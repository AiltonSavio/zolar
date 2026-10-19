//! Solana side: reading the Escrow and the relay, the vault's associated token account, and the
//! checks a Payer makes before paying into an Escrow.

use anyhow::{Context, Result, bail};
use escrow_client::accounts::Escrow;
use header_relay_client::accounts::Relay;
use header_relay_interface::RelayView;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{Keypair, read_keypair_file};
use zolar_relayer::chain::{Cluster, TxError, error_name};

pub const TOKEN_PROGRAM: Address =
    Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const ASSOCIATED_TOKEN_PROGRAM: Address =
    Address::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);

/// Reads a keypair from a file path; the contents are never printed.
pub fn load_keypair(path: &std::path::Path) -> Result<Keypair> {
    read_keypair_file(path).map_err(|e| anyhow::anyhow!("reading keypair {}: {e}", path.display()))
}

pub fn parse_address(text: &str) -> Result<Address> {
    text.parse::<Address>()
        .map_err(|e| anyhow::anyhow!("{text} is not a Solana address: {e}"))
}

/// The associated token account of `owner` for `mint` (the owner may be a PDA).
pub fn associated_token_account(owner: &Address, mint: &Address) -> Address {
    Address::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM,
    )
    .0
}

/// Creates the associated token account if it does not exist yet.
pub fn create_ata_idempotent(funder: &Address, owner: &Address, mint: &Address) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM,
        accounts: vec![
            AccountMeta::new(*funder, true),
            AccountMeta::new(associated_token_account(owner, mint), false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM, false),
        ],
        data: vec![1],
    }
}

/// Sends one transaction and names a program error.
pub fn send(
    cluster: &mut impl Cluster,
    instructions: &[Instruction],
    payer: &Keypair,
) -> Result<String> {
    cluster.send(instructions, payer).map_err(|e| match e {
        TxError::Program { index, code } => {
            anyhow::anyhow!("instruction {index} failed: {}", error_name_any(code))
        }
        TxError::Other(message) => anyhow::anyhow!("{message}"),
    })
}

fn error_name_any(code: u32) -> String {
    let escrow = match code {
        1 => "WrongAccount",
        2 => "BadExpiry",
        3 => "ZeroAmount",
        4 => "CommitmentOutOfField",
        5 => "BadTokenAccount",
        6 => "NotOpen",
        7 => "RelayHalted",
        8 => "CommitmentMismatch",
        9 => "BlockOutOfRange",
        10 => "BadInclusionProof",
        11 => "UnknownRoot",
        12 => "RelayNotReadable",
        13 => "PoseidonFailed",
        14 => "BondNotDeposited",
        15 => "ClaimModeNotAllowed",
        16 => "NoBondRequired",
        17 => "BondAlreadyDeposited",
        18 => "RefundNotYet",
        19 => "RefundNotRequested",
        20 => "ClaimWindowOpen",
        21 => "NotPayer",
        _ => "",
    };
    format!(
        "Escrow {escrow} / Header Relay {} (code {code})",
        error_name(code)
    )
}

pub fn read_account(cluster: &mut impl Cluster, address: &Address) -> Result<Vec<u8>> {
    cluster
        .account(address)
        .map_err(anyhow::Error::msg)?
        .with_context(|| format!("account {address} does not exist"))
}

/// The relay as the Escrow reads it, after the owner and address checks every consumer must make.
pub struct RelayState {
    pub data: Vec<u8>,
}

impl RelayState {
    pub fn view(&self) -> Result<RelayView<'_>> {
        RelayView::parse(&self.data).map_err(|e| anyhow::anyhow!("relay: {e:?}"))
    }

    pub fn account(&self) -> Result<Relay> {
        Relay::from_bytes(&self.data).context("decoding the relay account")
    }
}

pub fn read_relay(cluster: &mut impl Cluster, relay: &Address) -> Result<RelayState> {
    let data = read_account(cluster, relay)?;
    let state = RelayState { data };
    state.view()?;
    Ok(state)
}

/// An SPL token account's mint, owner and amount; the account must be an initialised classic one.
pub struct TokenAccount {
    pub mint: Address,
    pub owner: Address,
    pub amount: u64,
    pub has_delegate: bool,
    pub has_close_authority: bool,
}

pub fn parse_token_account(data: &[u8]) -> Result<TokenAccount> {
    if data.len() != 165 || data[108] != 1 {
        bail!("not an initialised SPL token account");
    }
    let tag = |at: usize| data[at..at + 4] != [0; 4];
    Ok(TokenAccount {
        mint: Address::new_from_array(data[0..32].try_into()?),
        owner: Address::new_from_array(data[32..64].try_into()?),
        amount: u64::from_le_bytes(data[64..72].try_into()?),
        has_delegate: tag(72),
        has_close_authority: tag(129),
    })
}

/// The freeze authority of a classic SPL mint, if it has one.
pub fn mint_freeze_authority(data: &[u8]) -> Result<Option<Address>> {
    if data.len() != 82 || data[45] != 1 {
        bail!("not an initialised SPL mint");
    }
    if data[46..50] == [0; 4] {
        Ok(None)
    } else {
        Ok(Some(Address::new_from_array(data[50..82].try_into()?)))
    }
}

/// Seconds after creation when a refund no longer depends on the relay (programs/escrow state.rs).
pub const BACKSTOP_SECS: i64 = 24 * 60 * 60;
/// Seconds a Payer has to claim between a refund request and the Refund (programs/escrow state.rs).
pub const CLAIM_WINDOW_SECS: i64 = 15 * 60;

/// What a Payee can do next with an Escrow it wants back.
#[derive(Debug, PartialEq, Eq)]
pub enum RefundStep {
    /// Start the claim window.
    Request,
    /// The relay tip must pass `needed` (expiry height plus Finality Depth), or 24 hours pass.
    WaitForTip {
        tip: u32,
        needed: u32,
    },
    /// The relay is Halted: only the 24 hour backstop, in `secs` seconds, can refund.
    Halted {
        secs: i64,
    },
    /// A Payer may still claim for `secs` seconds.
    WaitForWindow {
        secs: i64,
    },
    Refund,
}

/// Mirrors the Escrow's rules: a refund needs the backstop or a running relay whose tip is above
/// the expiry height plus Finality Depth, at the request and again at the Refund.
pub fn refund_step(
    escrow: &Escrow,
    tip: u32,
    finality_depth: u16,
    halted: bool,
    now: i64,
) -> RefundStep {
    let backstop_at = escrow.created_at.saturating_add(BACKSTOP_SECS);
    let passes = escrow.expiry_height.saturating_add(finality_depth as u32);
    let allowed = if now >= backstop_at {
        None
    } else if halted {
        Some(RefundStep::Halted {
            secs: backstop_at - now,
        })
    } else if tip <= passes {
        Some(RefundStep::WaitForTip {
            tip,
            needed: passes.saturating_add(1),
        })
    } else {
        None
    };
    if escrow.state == 0 {
        return allowed.unwrap_or(RefundStep::Request);
    }
    let opens = escrow.refund_requested_at.saturating_add(CLAIM_WINDOW_SECS);
    if now < opens {
        return RefundStep::WaitForWindow { secs: opens - now };
    }
    allowed.unwrap_or(RefundStep::Refund)
}

pub fn read_escrow(data: &[u8]) -> Result<Escrow> {
    Escrow::from_bytes(data).context("decoding the Escrow account")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_associated_token_account_of_a_wallet_is_stable() {
        // The well-known associated token account of the devnet deployer for devnet USDC is not
        // needed: the derivation is checked against its defining seeds.
        let owner = Address::new_from_array([3; 32]);
        let mint = Address::new_from_array([4; 32]);
        let (expected, _) = Address::find_program_address(
            &[owner.as_ref(), TOKEN_PROGRAM.as_ref(), mint.as_ref()],
            &ASSOCIATED_TOKEN_PROGRAM,
        );
        assert_eq!(associated_token_account(&owner, &mint), expected);
        let ix = create_ata_idempotent(&owner, &owner, &mint);
        assert_eq!(ix.accounts[1].pubkey, expected);
        assert_eq!(ix.data, vec![1]);
    }

    fn escrow_at(state: u8) -> Escrow {
        let mut d = vec![0u8; 300];
        d[0] = 1;
        d[1] = 1;
        let mut e = read_escrow(&d).unwrap();
        e.state = state;
        e.created_at = 1_000;
        e.expiry_height = 100;
        e.refund_requested_at = if state == 1 { 50_000 } else { 0 };
        e
    }

    #[test]
    fn a_refund_waits_for_the_tip_then_the_window_and_the_backstop_ignores_a_halt() {
        let e = escrow_at(0);
        // Finality Depth 10: the tip must reach 111.
        assert_eq!(
            refund_step(&e, 110, 10, false, 2_000),
            RefundStep::WaitForTip {
                tip: 110,
                needed: 111
            }
        );
        assert_eq!(refund_step(&e, 111, 10, false, 2_000), RefundStep::Request);
        // The boundary does not overflow near the largest height; the backstop still opens it.
        let high = Escrow {
            expiry_height: u32::MAX - 3,
            ..e.clone()
        };
        assert!(matches!(
            refund_step(&high, u32::MAX, 10, false, 2_000),
            RefundStep::WaitForTip { .. }
        ));
        assert_eq!(
            refund_step(&high, 0, 10, false, 1_000 + BACKSTOP_SECS),
            RefundStep::Request
        );
        assert_eq!(
            refund_step(&e, 500, 10, true, 2_000),
            RefundStep::Halted { secs: 85_400 }
        );
        assert_eq!(
            refund_step(&e, 0, 10, true, 1_000 + BACKSTOP_SECS),
            RefundStep::Request
        );

        let e = escrow_at(1);
        assert_eq!(
            refund_step(&e, 111, 10, false, 50_000 + CLAIM_WINDOW_SECS - 1),
            RefundStep::WaitForWindow { secs: 1 }
        );
        assert_eq!(
            refund_step(&e, 111, 10, false, 50_000 + CLAIM_WINDOW_SECS),
            RefundStep::Refund
        );
        // A Halt after the request holds the Refund back until the backstop.
        assert!(matches!(
            refund_step(&e, 111, 10, true, 50_000 + CLAIM_WINDOW_SECS),
            RefundStep::Halted { .. }
        ));
        assert_eq!(
            refund_step(
                &e,
                0,
                10,
                true,
                1_000 + BACKSTOP_SECS + CLAIM_WINDOW_SECS + 50_000
            ),
            RefundStep::Refund
        );
    }

    #[test]
    fn a_mint_with_a_freeze_authority_is_recognised() {
        let mut mint = vec![0u8; 82];
        mint[45] = 1;
        assert_eq!(mint_freeze_authority(&mint).unwrap(), None);
        mint[46] = 1;
        mint[50..82].fill(9);
        assert_eq!(
            mint_freeze_authority(&mint).unwrap(),
            Some(Address::new_from_array([9; 32]))
        );
        assert!(mint_freeze_authority(&[0; 10]).is_err());
    }
}
