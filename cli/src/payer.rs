//! The Payer's side: build the PCZT, check the Escrow before paying, pay, wait for finality, claim.

use std::{
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use escrow_client::{
    accounts::Escrow,
    instructions::{DepositBondBuilder, RevealClaimBuilder},
};
use solana_address::Address;
use solana_signer::Signer;
use zolar_claim::{FinalBlockTree, block_leaf, tx_merkle_path};
use zolar_pczt::summary;
use zolar_relayer::{chain::Cluster, p2p, rpc::HttpRpc};

use crate::{
    blocks::{fetch_headers, merkle_root, verified_block_txids},
    devtool::Devtool,
    explorer::Explorer,
    internal_txid,
    rpc::finalized_account_within,
    solana::{
        RelayState, associated_token_account, create_ata_idempotent, load_keypair,
        mint_freeze_authority, parse_token_account, read_account, read_escrow, read_relay, send,
    },
};

const POLL: Duration = Duration::from_secs(10);

pub struct Chains {
    pub rpc_url: String,
    pub explorer_url: String,
    pub relay: Address,
    pub peers: Vec<String>,
}

/// Builds the unsigned PCZT: the Payee's address, a value, the challenge memo, and an expiry
/// `expiry_blocks` above the relay tip so the Escrow's 20 block limit holds.
pub fn create_pczt(
    chains: &Chains,
    devtool: &Devtool,
    address: &str,
    value: u64,
    memo: &str,
    expiry_blocks: u32,
    out: &Path,
) -> Result<()> {
    ensure!(
        (1..=20).contains(&expiry_blocks),
        "--expiry-blocks must be 1 to 20: the Escrow refuses an expiry further ahead"
    );
    let mut rpc = HttpRpc::new(&chains.rpc_url);
    let relay = read_relay(&mut rpc, &chains.relay)?;
    let tip = relay.view()?.tip_height();
    println!("syncing the wallet");
    devtool.sync()?;
    let expiry = tip + expiry_blocks;
    devtool.create(address, value, memo, expiry, out)?;
    let s = summary(&std::fs::read(out)?)?;
    println!("PCZT written to {}", out.display());
    println!("transaction ID : {}", s.txid);
    println!("expiry height  : {} (relay tip {tip})", s.expiry_height);
    Ok(())
}

/// What the Payer learned about an Escrow it is willing to pay into.
pub struct Checked {
    pub escrow: Escrow,
    pub escrow_address: Address,
    pub txid: String,
    pub expiry_height: u32,
}

pub struct Check<'a> {
    pub chains: &'a Chains,
    pub escrow_address: Address,
    pub payer_wallet: Address,
    pub pczt: &'a Path,
    pub salt: [u8; 32],
    pub accept_freeze_authority: bool,
    /// The token the Payer agreed to be paid in. The Payee's Escrow must hold exactly this mint.
    pub expected_mint: Address,
    /// The least tokens (smallest unit) the Payer agreed to be paid.
    pub min_amount: u64,
    /// The most the Payer agreed to lock as a Bond.
    pub max_bond: u64,
    /// How long to wait for the Escrow to be finalized on Solana.
    pub finalized_wait: Duration,
}

/// The terms of the trade as the Payer agreed them, against what the Payee locked.
pub fn check_terms(
    escrow: &Escrow,
    payer_wallet: &Address,
    relay: &Address,
    expected_mint: &Address,
    min_amount: u64,
    max_bond: u64,
) -> Result<()> {
    ensure!(
        escrow.state == 0,
        "the Escrow is not open (a refund may be under way)"
    );
    ensure!(
        &escrow.relay == relay,
        "the Escrow trusts relay {}, not the expected {relay}",
        escrow.relay
    );
    ensure!(
        &escrow.payout == payer_wallet,
        "the Escrow pays {}, not this Payer wallet {payer_wallet}",
        escrow.payout
    );
    ensure!(
        &escrow.mint == expected_mint,
        "the Escrow holds mint {}, not the agreed {expected_mint}",
        escrow.mint
    );
    ensure!(
        escrow.amount >= min_amount,
        "the Escrow locks {} tokens, fewer than the agreed {min_amount}",
        escrow.amount
    );
    ensure!(
        escrow.bond_required <= max_bond,
        "the Escrow asks for a Bond of {}, more than the agreed {max_bond}",
        escrow.bond_required
    );
    Ok(())
}

/// Everything a Payer verifies before signing. Read-only.
pub fn check_escrow(c: &Check) -> Result<Checked> {
    let mut rpc = HttpRpc::new(&c.chains.rpc_url);
    let s = summary(&std::fs::read(c.pczt)?)?;
    let url = &c.chains.rpc_url;
    let data = finalized_account_within(url, &c.escrow_address, c.finalized_wait)?;
    let escrow = read_escrow(&data)?;

    // Only the Escrow program can create the account at this address, so its contents are its own.
    ensure!(
        Escrow::find_pda(&escrow.payee, escrow.commitment).0 == c.escrow_address,
        "{} is not an Escrow account of the Escrow program",
        c.escrow_address
    );
    check_terms(
        &escrow,
        &c.payer_wallet,
        &c.chains.relay,
        &c.expected_mint,
        c.min_amount,
        c.max_bond,
    )?;
    let commitment = zolar_claim::payment_commitment(&internal_txid(&s.txid)?, &c.salt)?;
    ensure!(
        commitment == escrow.commitment,
        "the Escrow's Payment Commitment does not match this PCZT's transaction ID and the salt"
    );
    ensure!(
        escrow.expiry_height == s.expiry_height,
        "the Escrow expires at {} but the transaction at {}: a payment mined in between would be lost",
        escrow.expiry_height,
        s.expiry_height
    );

    let vault = parse_token_account(&finalized_account_within(
        url,
        &escrow.vault,
        c.finalized_wait,
    )?)?;
    ensure!(
        vault.owner == c.escrow_address && vault.mint == escrow.mint,
        "the vault is not a token account of the Escrow for its mint"
    );
    ensure!(
        vault.amount >= escrow.amount && !vault.has_delegate && !vault.has_close_authority,
        "the vault does not hold the tokens or has a delegate or close authority"
    );

    let mint = finalized_account_within(url, &escrow.mint, c.finalized_wait)?;
    match mint_freeze_authority(&mint)? {
        Some(authority) if !c.accept_freeze_authority => bail!(
            "the mint has a freeze authority ({authority}); it could freeze the vault after you pay. \
             Pass --accept-freeze-authority if you trust it"
        ),
        Some(authority) => println!("warning: the mint has a freeze authority ({authority})"),
        None => println!("the mint has no freeze authority"),
    }

    let relay = read_relay(&mut rpc, &escrow.relay)?;
    let view = relay.view()?;
    view.ensure_running()
        .map_err(|e| anyhow::anyhow!("the relay is halted: {e:?}"))?;
    ensure!(
        view.tip_height() < s.expiry_height,
        "the relay tip {} is already at the expiry {}",
        view.tip_height(),
        s.expiry_height
    );

    println!("Escrow {} checked", c.escrow_address);
    println!("  mint            : {}", escrow.mint);
    println!(
        "  amount          : {} (vault holds {})",
        escrow.amount, vault.amount
    );
    println!("  payout          : {}", escrow.payout);
    println!(
        "  Bond            : {} required, {} deposited",
        escrow.bond_required, escrow.bond_deposited
    );
    println!("  transaction ID  : {}", s.txid);
    println!(
        "  expiry          : {} (relay tip {}, Finality Depth {})",
        s.expiry_height,
        view.tip_height(),
        view.finality_depth()
    );
    Ok(Checked {
        txid: s.txid,
        expiry_height: s.expiry_height,
        escrow,
        escrow_address: c.escrow_address,
    })
}

/// Locks the Bond the Escrow requires, if it is not locked yet. A Payer calls this before broadcasting:
/// a Reveal Claim cannot settle without it, and it returns with the tokens.
pub fn deposit_bond(chains: &Chains, keypair: &Path, escrow_address: Address) -> Result<()> {
    let mut rpc = HttpRpc::new(&chains.rpc_url);
    let payer = load_keypair(keypair)?;
    let escrow = read_escrow(&read_account(&mut rpc, &escrow_address)?)?;
    ensure!(
        escrow.payout == payer.pubkey(),
        "the Escrow pays {}, not this keypair; only that wallet can lock the Bond",
        escrow.payout
    );
    if escrow.bond_required == 0 {
        println!("the Escrow requires no Bond");
        return Ok(());
    }
    if escrow.bond_deposited >= escrow.bond_required {
        println!("the Bond of {} is already locked", escrow.bond_deposited);
        return Ok(());
    }
    let source = associated_token_account(&payer.pubkey(), &escrow.mint);
    let held = parse_token_account(&read_account(&mut rpc, &source)?)
        .context("the Payer has no token account for the Escrow's mint, so no tokens for the Bond")?
        .amount;
    ensure!(
        held >= escrow.bond_required,
        "the Payer holds {held} tokens, fewer than the {} Bond",
        escrow.bond_required
    );
    let ix = DepositBondBuilder::new()
        .payer(payer.pubkey())
        .escrow(escrow_address)
        .payer_token(source)
        .vault(escrow.vault)
        .instruction();
    let signature = send(&mut rpc, &[ix], &payer)?;
    println!(
        "Bond locked         : {signature} ({} tokens)",
        escrow.bond_required
    );
    Ok(())
}

/// Proves, signs and broadcasts the checked PCZT, then waits until the explorer shows it mined.
/// Returns the block height.
pub fn pay(
    chains: &Chains,
    devtool: &Devtool,
    checked: &Checked,
    pczt: &Path,
    work: &Path,
) -> Result<u32> {
    std::fs::create_dir_all(work)?;
    let proven = work.join("proven.pczt");
    let signed = work.join("signed.pczt");
    let combined = work.join("combined.pczt");
    println!("proving");
    devtool.prove(pczt, &proven)?;
    println!("signing");
    devtool.sign(&proven, &signed)?;
    devtool.combine(&proven, &signed, &combined)?;
    ensure!(
        summary(&std::fs::read(&combined)?)?.txid == checked.txid,
        "the signed transaction has another ID than the committed one"
    );
    println!("broadcasting {}", checked.txid);
    devtool.send(&combined)?;
    wait_until_mined(chains, &checked.txid)
}

pub fn wait_until_mined(chains: &Chains, txid: &str) -> Result<u32> {
    let explorer = Explorer::new(&chains.explorer_url);
    let started = Instant::now();
    loop {
        if let Some(mined) = explorer.mined(txid)? {
            println!("mined in block {}", mined.block_height);
            return Ok(mined.block_height);
        }
        ensure!(
            started.elapsed() < Duration::from_secs(30 * 60),
            "transaction {txid} was not mined in 30 minutes"
        );
        sleep(POLL);
    }
}

/// The Final Block Tree rebuilt from Zcash P2P headers, proven equal to the relay's.
pub struct Rebuilt {
    pub headers: Vec<p2p::RawHeader>,
    pub tree: FinalBlockTree,
    pub root: [u8; 32],
}

/// Rebuilds the relay's Final Block Tree from the headers of the blocks it holds as final and checks
/// the root against the relay's own. A mismatch means the peers' chain is not the relay's.
pub fn rebuild_tree(chains: &Chains, relay: &RelayState) -> Result<Rebuilt> {
    let view = relay.view()?;
    let checkpoint = relay.account()?.checkpoint;
    let size = view.leaf_count() as usize;
    let headers = fetch_headers(checkpoint.hash, size, &chains.peers)?;
    let leaves = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            Ok(block_leaf(
                checkpoint.height + 1 + i as u32,
                &merkle_root(h),
            )?)
        })
        .collect::<Result<Vec<_>>>()?;
    let tree = FinalBlockTree::new(leaves);
    let root = tree.root(size);
    ensure!(
        view.root_at(size as u32) == Some(root),
        "the tree rebuilt from Zcash P2P headers does not equal the relay's root: the peers' chain \
         differs from the relay's"
    );
    println!(
        "rebuilt {size} leaves from P2P headers; the root {} equals the relay's",
        hex::encode(root)
    );
    Ok(Rebuilt {
        headers,
        tree,
        root,
    })
}

pub struct Claim<'a> {
    pub chains: &'a Chains,
    pub keypair: PathBuf,
    pub escrow_address: Address,
    pub txid: &'a str,
    pub block_height: u32,
    pub salt: [u8; 32],
}

/// Waits until the block is in the Final Block Tree, builds the Inclusion Proof and the tree path from
/// data outside Solana, checks both against the relay's own root, and sends the Reveal Claim.
pub fn reveal_claim(c: &Claim) -> Result<String> {
    let mut rpc = HttpRpc::new(&c.chains.rpc_url);
    let payer = load_keypair(&c.keypair)?;
    let escrow = read_escrow(&read_account(&mut rpc, &c.escrow_address)?)?;

    let started = Instant::now();
    let relay = loop {
        let relay = read_relay(&mut rpc, &c.chains.relay)?;
        let view = relay.view()?;
        view.ensure_running()
            .map_err(|e| anyhow::anyhow!("the relay is halted: {e:?}"))?;
        if view.finalized_height() >= c.block_height {
            break relay;
        }
        println!(
            "waiting for finality: block {}, relay tip {}, final up to {}",
            c.block_height,
            view.tip_height(),
            view.finalized_height()
        );
        ensure!(
            started.elapsed() < Duration::from_secs(30 * 60),
            "the block did not become final in 30 minutes"
        );
        sleep(POLL);
    };
    let view = relay.view()?;
    let checkpoint = relay.account()?.checkpoint;
    let size = view.leaf_count() as usize;
    let index = (c.block_height - checkpoint.height - 1) as usize;
    ensure!(index < size, "the block is not in the tree yet");
    println!("final block tree: {size} leaves, the block is leaf {index}");
    let Rebuilt {
        headers,
        tree,
        root,
    } = rebuild_tree(c.chains, &relay)?;

    let txids = verified_block_txids(
        &Explorer::new(&c.chains.explorer_url),
        c.block_height,
        &merkle_root(&headers[index]),
    )?;
    let me = internal_txid(c.txid)?;
    let tx_index = txids
        .iter()
        .position(|t| *t == me)
        .with_context(|| format!("{} is not in block {}", c.txid, c.block_height))?;
    println!(
        "Inclusion Proof: transaction {tx_index} of {} in block {}",
        txids.len(),
        c.block_height
    );

    let payout_token = associated_token_account(&payer.pubkey(), &escrow.mint);
    let claim = RevealClaimBuilder::new()
        .escrow(c.escrow_address)
        .relay(c.chains.relay)
        .vault(escrow.vault)
        .payout_token(payout_token)
        .payee(escrow.payee)
        .txid(me)
        .salt(c.salt)
        .block_height(c.block_height)
        .tx_index(tx_index as u32)
        .root(root)
        .tree_path(tree.compress(tree.path(index, size)).into())
        .tx_path(tx_merkle_path(&txids, tx_index).into())
        .instruction();
    // The Payer's token account is made in its own transaction: with it the claim would not fit one.
    if rpc
        .account(&payout_token)
        .map_err(anyhow::Error::msg)?
        .is_none()
    {
        let ata = create_ata_idempotent(&payer.pubkey(), &payer.pubkey(), &escrow.mint);
        let created = send(&mut rpc, &[ata], &payer)?;
        println!("Payer token account : {payout_token} ({created})");
    }
    let signature = send(&mut rpc, &[claim], &payer)?;
    println!("Reveal Claim        : {signature}");
    Ok(signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escrow() -> Escrow {
        Escrow {
            state: 0,
            payee: Address::new_from_array([1; 32]),
            payout: Address::new_from_array([2; 32]),
            mint: Address::new_from_array([3; 32]),
            relay: Address::new_from_array([4; 32]),
            amount: 5_000_000,
            ..Escrow::from_bytes(&sample()).unwrap()
        }
    }

    fn sample() -> Vec<u8> {
        let mut d = vec![0u8; 300];
        d[0] = 1;
        d[1] = 1;
        d
    }

    #[test]
    fn the_payer_pays_only_into_the_agreed_mint_and_amount() {
        let e = escrow();
        let (payer, relay, mint) = (e.payout, e.relay, e.mint);
        check_terms(&e, &payer, &relay, &mint, 5_000_000, 0).unwrap();
        check_terms(&e, &payer, &relay, &mint, 1, 0).unwrap();
        // A worthless token locked against the right payout and relay.
        let other = Address::new_from_array([9; 32]);
        assert!(check_terms(&e, &payer, &relay, &other, 1, 0).is_err());
        // Underfunded.
        assert!(check_terms(&e, &payer, &relay, &mint, 5_000_001, 0).is_err());
        // Another payout wallet, another relay, a required Bond, a closed Escrow.
        assert!(check_terms(&e, &other, &relay, &mint, 1, 0).is_err());
        assert!(check_terms(&e, &payer, &other, &mint, 1, 0).is_err());
        // A Bond above the agreed maximum is refused, one within it accepted.
        let with_bond = Escrow {
            bond_required: 100,
            ..e.clone()
        };
        assert!(check_terms(&with_bond, &payer, &relay, &mint, 1, 99).is_err());
        assert!(check_terms(&with_bond, &payer, &relay, &mint, 1, 100).is_ok());
        assert!(check_terms(&with_bond, &payer, &relay, &mint, 1, 0).is_err());
        let closed = Escrow { state: 1, ..e };
        assert!(check_terms(&closed, &payer, &relay, &mint, 1, 0).is_err());
    }
}
