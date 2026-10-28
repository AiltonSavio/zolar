use groth16_solana::groth16::Groth16Verifier;
use header_relay_interface::{leaf_inputs, RelayView, ViewError, TREE_DEPTH};
use pinocchio::{
    cpi::{invoke_signed, Seed, Signer},
    error::ProgramError,
    instruction::{InstructionAccount, InstructionView},
    sysvars::{clock::Clock, rent::Rent, Sysvar},
    AccountView, Address, ProgramResult,
};

use crate::{error::EscrowError, state::*, verifying_key, zeros::ZEROS};

const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);
const TOKEN: Address = Address::new_from_array(TOKEN_PROGRAM);

fn fail(e: EscrowError) -> ProgramError {
    ProgramError::Custom(e as u32)
}

/// Solana syscalls. The program only runs on Solana; the host versions let it type-check and lint
/// on a developer machine and are never called.
#[cfg(target_os = "solana")]
mod sys {
    use pinocchio::{error::ProgramError, Address};

    use crate::error::EscrowError;

    pub fn sha256(data: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let vals: [&[u8]; 1] = [data];
        // SAFETY: `vals` is a one-element slice-of-slices, the layout the syscall reads.
        unsafe {
            pinocchio::syscalls::sol_sha256(vals.as_ptr() as *const u8, 1, out.as_mut_ptr());
        }
        out
    }

    pub fn find_program_address(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
        Address::find_program_address(seeds, program_id)
    }

    /// Poseidon over BN254 (circom parameters), big-endian; every input is a 32-byte field element.
    pub fn poseidon(inputs: &[&[u8; 32]]) -> Result<[u8; 32], ProgramError> {
        let mut vals = [&[0u8; 0][..]; 3];
        for (v, i) in vals.iter_mut().zip(inputs) {
            *v = &i[..];
        }
        let mut out = [0u8; 32];
        // SAFETY: `vals` holds `inputs.len()` (ptr, len) slices, the layout the syscall reads.
        let code = unsafe {
            pinocchio::syscalls::sol_poseidon(
                0,
                0,
                vals.as_ptr() as *const u8,
                inputs.len() as u64,
                out.as_mut_ptr(),
            )
        };
        if code == 0 {
            Ok(out)
        } else {
            Err(ProgramError::Custom(EscrowError::PoseidonFailed as u32))
        }
    }
}

#[cfg(not(target_os = "solana"))]
mod sys {
    use pinocchio::{error::ProgramError, Address};

    pub fn sha256(_: &[u8]) -> [u8; 32] {
        unreachable!("the Escrow runs only on Solana")
    }

    pub fn find_program_address(_: &[&[u8]], _: &Address) -> (Address, u8) {
        unreachable!("the Escrow runs only on Solana")
    }

    pub fn poseidon(_: &[&[u8; 32]]) -> Result<[u8; 32], ProgramError> {
        unreachable!("the Escrow runs only on Solana")
    }
}

use sys::{find_program_address, poseidon, sha256};

/// Groth16 over BN254 with the Private Claim verifying key. `proof_a` is already negated.
fn verify_proof(
    proof_a: &[u8; 64],
    proof_b: &[u8; 128],
    proof_c: &[u8; 64],
    public_inputs: &[[u8; 32]; verifying_key::PUBLIC_INPUTS],
) -> Result<(), ProgramError> {
    let invalid = |_| fail(EscrowError::InvalidProof);
    Groth16Verifier::<{ verifying_key::PUBLIC_INPUTS }>::new(
        proof_a,
        proof_b,
        proof_c,
        public_inputs,
        &verifying_key::VERIFYINGKEY,
    )
    .map_err(invalid)?
    .verify()
    .map_err(invalid)
}

fn sha256d_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut data = [0u8; 64];
    data[..32].copy_from_slice(left);
    data[32..].copy_from_slice(right);
    sha256(&sha256(&data))
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}
fn i64_at(d: &[u8], at: usize) -> i64 {
    u64_at(d, at) as i64
}
fn u64_at(d: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[at..at + 8]);
    u64::from_le_bytes(b)
}
fn array32(d: &[u8], at: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&d[at..at + 32]);
    out
}
fn put(d: &mut [u8], at: usize, bytes: &[u8]) {
    d[at..at + bytes.len()].copy_from_slice(bytes);
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (&tag, args) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match tag {
        0 => create_escrow(program_id, accounts, args),
        1 => reveal_claim(program_id, accounts, args),
        2 => deposit_bond(program_id, accounts, args),
        3 => refund(program_id, accounts, args),
        4 => private_claim(program_id, accounts, args),
        5 => request_refund(program_id, accounts, args),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

/// The view of `relay`, which must be owned by the Header Relay program, be `expected` (the address the
/// Escrow trusts), sealed and running.
fn relay_view<'a>(
    relay: &AccountView,
    expected: &[u8; 32],
    data: &'a [u8],
) -> Result<RelayView<'a>, ProgramError> {
    // SAFETY: nothing assigns or closes the account while this copy is read.
    let owner: [u8; 32] = unsafe { *relay.owner().as_array() };
    RelayView::parse(&owner, relay.address().as_array(), expected, data).map_err(|e| match e {
        ViewError::WrongOwner | ViewError::WrongAddress => fail(EscrowError::WrongAccount),
        ViewError::Halted(_) => fail(EscrowError::RelayHalted),
        ViewError::NotARelay | ViewError::NotSealed => fail(EscrowError::RelayNotReadable),
    })
}

/// The relay address stored in the Escrow.
fn stored_relay(escrow: &AccountView) -> Result<[u8; 32], ProgramError> {
    Ok(array32(&escrow.try_borrow()?, E_RELAY))
}

// Token account layout (classic SPL Token).
const T_MINT: usize = 0;
const T_OWNER: usize = 32;
const T_AMOUNT: usize = 64;
const T_DELEGATE_TAG: usize = 72;
const T_STATE: usize = 108;
const T_NATIVE_TAG: usize = 109;
const T_CLOSE_TAG: usize = 129;
const T_LEN: usize = 165;
const T_INITIALIZED: u8 = 1;

/// Fields of a token account the Escrow cares about.
struct Token {
    mint: [u8; 32],
    owner: [u8; 32],
    amount: u64,
}

/// An initialised, unfrozen classic SPL Token account. With `strict`, also one with no delegate, no
/// close authority and not wrapped SOL: what a vault must be, because a delegate could move its tokens
/// and a close authority could close it. A destination only has to be able to receive.
fn read_token(account: &AccountView, strict: bool) -> Result<Token, ProgramError> {
    if !account.owned_by(&TOKEN) {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    let d = account.try_borrow()?;
    let tag = |at: usize| d[at] | d[at + 1] | d[at + 2] | d[at + 3];
    if d.len() != T_LEN || d[T_STATE] != T_INITIALIZED {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    if strict && (tag(T_DELEGATE_TAG) != 0 || tag(T_NATIVE_TAG) != 0 || tag(T_CLOSE_TAG) != 0) {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    Ok(Token {
        mint: array32(&d, T_MINT),
        owner: array32(&d, T_OWNER),
        amount: u64_at(&d, T_AMOUNT),
    })
}

fn token_transfer(
    from: &AccountView,
    to: &AccountView,
    authority: &AccountView,
    amount: u64,
    signers: &[Signer],
) -> ProgramResult {
    let mut data = [0u8; 9];
    data[0] = 3;
    data[1..].copy_from_slice(&amount.to_le_bytes());
    invoke_signed(
        &InstructionView {
            program_id: &TOKEN,
            data: &data,
            accounts: &[
                InstructionAccount::writable(from.address()),
                InstructionAccount::writable(to.address()),
                InstructionAccount::readonly_signer(authority.address()),
            ],
        },
        &[from, to, authority],
        signers,
    )
}

fn token_close(
    account: &AccountView,
    destination: &AccountView,
    authority: &AccountView,
    signers: &[Signer],
) -> ProgramResult {
    invoke_signed(
        &InstructionView {
            program_id: &TOKEN,
            data: &[9],
            accounts: &[
                InstructionAccount::writable(account.address()),
                InstructionAccount::writable(destination.address()),
                InstructionAccount::readonly_signer(authority.address()),
            ],
        },
        &[account, destination, authority],
        signers,
    )
}

fn require_signer(account: &AccountView) -> ProgramResult {
    if account.is_signer() {
        Ok(())
    } else {
        Err(ProgramError::MissingRequiredSignature)
    }
}

fn check_program(account: &AccountView, id: &Address) -> ProgramResult {
    if account.address() == id {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}

/// Creates the Escrow PDA, also when someone already sent it lamports.
fn create_escrow_account(
    program_id: &Address,
    payer: &AccountView,
    account: &AccountView,
    seeds: &[&[u8]; 3],
) -> Result<u8, ProgramError> {
    let (pda, bump) = find_program_address(seeds, program_id);
    if account.address() != &pda {
        return Err(fail(EscrowError::WrongAccount));
    }
    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(seeds[0]),
        Seed::from(seeds[1]),
        Seed::from(seeds[2]),
        Seed::from(&bump_seed),
    ];
    let needed = Rent::get()?.try_minimum_balance(ESCROW_LEN)?;
    let have = account.lamports();
    if have == 0 {
        let mut data = [0u8; 52];
        data[4..12].copy_from_slice(&needed.to_le_bytes());
        data[12..20].copy_from_slice(&(ESCROW_LEN as u64).to_le_bytes());
        data[20..].copy_from_slice(program_id.as_ref());
        invoke_signed(
            &InstructionView {
                program_id: &SYSTEM_PROGRAM,
                data: &data,
                accounts: &[
                    InstructionAccount::writable_signer(payer.address()),
                    InstructionAccount::writable_signer(account.address()),
                ],
            },
            &[payer, account],
            &[Signer::from(&signer_seeds)],
        )?;
    } else {
        if have < needed {
            let mut data = [0u8; 12];
            data[..4].copy_from_slice(&2u32.to_le_bytes());
            data[4..].copy_from_slice(&(needed - have).to_le_bytes());
            invoke_signed(
                &InstructionView {
                    program_id: &SYSTEM_PROGRAM,
                    data: &data,
                    accounts: &[
                        InstructionAccount::writable_signer(payer.address()),
                        InstructionAccount::writable(account.address()),
                    ],
                },
                &[payer, account],
                &[],
            )?;
        }
        let mut allocate = [0u8; 12];
        allocate[..4].copy_from_slice(&8u32.to_le_bytes());
        allocate[4..].copy_from_slice(&(ESCROW_LEN as u64).to_le_bytes());
        invoke_signed(
            &InstructionView {
                program_id: &SYSTEM_PROGRAM,
                data: &allocate,
                accounts: &[InstructionAccount::writable_signer(account.address())],
            },
            &[account],
            &[Signer::from(&signer_seeds)],
        )?;
        let mut assign = [0u8; 36];
        assign[..4].copy_from_slice(&1u32.to_le_bytes());
        assign[4..].copy_from_slice(program_id.as_ref());
        invoke_signed(
            &InstructionView {
                program_id: &SYSTEM_PROGRAM,
                data: &assign,
                accounts: &[InstructionAccount::writable_signer(account.address())],
            },
            &[account],
            &[Signer::from(&signer_seeds)],
        )?;
    }
    Ok(bump)
}

const CREATE_ARGS_LEN: usize = 32 + 32 + 8 + 8 + 4 + 1;

fn create_escrow(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [payee, escrow, payee_token, vault, relay, token_program, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if args.len() != CREATE_ARGS_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let commitment = array32(args, 0);
    let payout = array32(args, 32);
    let amount = u64_at(args, 64);
    let bond_required = u64_at(args, 72);
    let expiry_height = u32_at(args, 80);
    let claim_mode = args[84];

    require_signer(payee)?;
    check_program(token_program, &TOKEN)?;
    check_program(system, &SYSTEM_PROGRAM)?;
    if amount == 0 {
        return Err(fail(EscrowError::ZeroAmount));
    }
    if claim_mode != MODE_REVEAL_ALLOWED && claim_mode != MODE_PRIVATE_ONLY {
        return Err(fail(EscrowError::ClaimModeNotAllowed));
    }
    if !below_modulus(&commitment) {
        return Err(fail(EscrowError::CommitmentOutOfField));
    }

    let tip = {
        let data = relay.try_borrow()?;
        let view = relay_view(relay, relay.address().as_array(), &data)?;
        view.tip_height()
    };
    if expiry_height <= tip || expiry_height > tip.saturating_add(MAX_EXPIRY_AHEAD) {
        return Err(fail(EscrowError::BadExpiry));
    }

    let source = read_token(payee_token, false)?;
    if source.owner != *payee.address().as_ref() || source.amount < amount {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    let locked = read_token(vault, true)?;
    if locked.mint != source.mint
        || locked.owner != *escrow.address().as_ref()
        || vault.address() == payee_token.address()
        || vault.address() == payee.address()
    {
        return Err(fail(EscrowError::BadTokenAccount));
    }

    let bump = create_escrow_account(
        program_id,
        payee,
        escrow,
        &[SEED_ESCROW, payee.address().as_ref(), &commitment],
    )?;
    token_transfer(payee_token, vault, payee, amount, &[])?;

    let now = Clock::get()?.unix_timestamp;
    let mut e = escrow.try_borrow_mut()?;
    e[E_DISC] = DISC_ESCROW;
    e[E_VERSION] = VERSION;
    e[E_STATE] = STATE_OPEN;
    e[E_BUMP] = bump;
    put(&mut e, E_PAYEE, payee.address().as_ref());
    put(&mut e, E_PAYOUT, &payout);
    put(&mut e, E_MINT, &source.mint);
    put(&mut e, E_VAULT, vault.address().as_ref());
    put(&mut e, E_RELAY, relay.address().as_ref());
    put(&mut e, E_COMMITMENT, &commitment);
    put(&mut e, E_AMOUNT, &amount.to_le_bytes());
    put(&mut e, E_BOND_REQUIRED, &bond_required.to_le_bytes());
    put(&mut e, E_CREATED_AT, &now.to_le_bytes());
    put(&mut e, E_CREATED_TIP, &tip.to_le_bytes());
    put(&mut e, E_EXPIRY_HEIGHT, &expiry_height.to_le_bytes());
    e[E_CLAIM_MODE] = claim_mode;
    Ok(())
}

/// Fixed part of the Reveal Claim data: txid, salt, height, index, root.
const CLAIM_FIXED_LEN: usize = 32 + 32 + 4 + 4 + 32;

/// Reads a `u8` count and that many 32-byte nodes from `args` at `at`; returns the nodes and the end.
fn read_path(args: &[u8], at: usize, max: usize) -> Result<(&[u8], usize), ProgramError> {
    let count = *args.get(at).ok_or(ProgramError::InvalidInstructionData)? as usize;
    if count > max {
        return Err(fail(EscrowError::BadInclusionProof));
    }
    let end = at + 1 + count * 32;
    let nodes = args
        .get(at + 1..end)
        .ok_or(ProgramError::InvalidInstructionData)?;
    Ok((nodes, end))
}

fn reveal_claim(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [escrow, relay, vault, payout_token, payee, token_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if args.len() < CLAIM_FIXED_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let txid = array32(args, 0);
    let salt = array32(args, 32);
    let block_height = u32_at(args, 64);
    let tx_index = u32_at(args, 68);
    let root = array32(args, 72);
    let (tree_path, at) = read_path(args, CLAIM_FIXED_LEN, TREE_DEPTH)?;
    let (tx_path, end) = read_path(args, at, MAX_TX_PATH)?;
    if end != args.len() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let tx_count = tx_path.len() / 32;

    check_program(token_program, &TOKEN)?;
    if !escrow.owned_by(program_id) || !escrow.is_writable() {
        return Err(fail(EscrowError::WrongAccount));
    }
    let (commitment, created_tip, expiry_height, stored_payout, mint) = {
        let e = escrow.try_borrow()?;
        if e.len() != ESCROW_LEN || e[E_DISC] != DISC_ESCROW || e[E_VERSION] != VERSION {
            return Err(fail(EscrowError::WrongAccount));
        }
        if e[E_STATE] != STATE_OPEN && e[E_STATE] != STATE_REFUND_REQUESTED {
            return Err(fail(EscrowError::NotOpen));
        }
        if e[E_CLAIM_MODE] != MODE_REVEAL_ALLOWED {
            return Err(fail(EscrowError::ClaimModeNotAllowed));
        }
        if u64_at(&e, E_BOND_DEPOSITED) < u64_at(&e, E_BOND_REQUIRED) {
            return Err(fail(EscrowError::BondNotDeposited));
        }
        if array32(&e, E_RELAY) != *relay.address().as_ref()
            || array32(&e, E_VAULT) != *vault.address().as_ref()
            || array32(&e, E_PAYEE) != *payee.address().as_ref()
        {
            return Err(fail(EscrowError::WrongAccount));
        }
        (
            array32(&e, E_COMMITMENT),
            u32_at(&e, E_CREATED_TIP),
            u32_at(&e, E_EXPIRY_HEIGHT),
            array32(&e, E_PAYOUT),
            array32(&e, E_MINT),
        )
    };

    let relay_data = relay.try_borrow()?;
    let view = relay_view(relay, &stored_relay(escrow)?, &relay_data)?;

    // The commitment: Poseidon over the two 128-bit limbs of the transaction ID and the salt.
    let [_, txid_hi, txid_lo] = leaf_inputs(0, &txid);
    if poseidon(&[&txid_hi, &txid_lo, &salt])? != commitment {
        return Err(fail(EscrowError::CommitmentMismatch));
    }

    // The block must not predate the Escrow by more than a replaceable stretch of chain, and must not
    // come after the expiry the payment carries.
    if !block_in_window(
        block_height,
        created_tip,
        expiry_height,
        view.finality_depth(),
    ) {
        return Err(fail(EscrowError::BlockOutOfRange));
    }
    let index = leaf_index(block_height, view.checkpoint_height())
        .ok_or(fail(EscrowError::BlockOutOfRange))?;

    // Inclusion Proof: SHA-256d path from the transaction to the block's Merkle root.
    let mut node = txid;
    for (level, sibling) in tx_path.chunks_exact(32).enumerate() {
        let sibling = array32(sibling, 0);
        node = if (tx_index >> level) & 1 == 0 {
            sha256d_pair(&node, &sibling)
        } else {
            sha256d_pair(&sibling, &node)
        };
    }
    if tx_index >> tx_count != 0 {
        return Err(fail(EscrowError::BadInclusionProof));
    }

    // Final Block Tree: the block's leaf up the Poseidon path to a recent root.
    let [height_field, root_hi, root_lo] = leaf_inputs(block_height, &node);
    let mut node = poseidon(&[&height_field, &root_hi, &root_lo])?;
    for (level, zero) in ZEROS.iter().enumerate() {
        let sibling = match tree_path.get(level * 32..level * 32 + 32) {
            Some(bytes) => array32(bytes, 0),
            None => *zero,
        };
        node = if (index >> level) & 1 == 0 {
            poseidon(&[&node, &sibling])?
        } else {
            poseidon(&[&sibling, &node])?
        };
    }
    if node != root {
        return Err(fail(EscrowError::UnknownRoot));
    }
    match view.root_size(&root) {
        Some(size) if index < size => {}
        _ => return Err(fail(EscrowError::UnknownRoot)),
    }
    drop(relay_data);

    // Pay the Payer wallet named at creation, whoever submitted the Claim.
    let paid = read_token(payout_token, false)?;
    if paid.mint != mint || paid.owner != stored_payout {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    settle(escrow, vault, payout_token, payee)
}

/// Pays the whole vault (tokens and Bond) to `destination`, closes the vault and the Escrow, and
/// returns all their rent to the Payee.
fn settle(
    escrow: &AccountView,
    vault: &AccountView,
    destination: &AccountView,
    payee: &AccountView,
) -> ProgramResult {
    let (bump, stored_payee, commitment) = {
        let e = escrow.try_borrow()?;
        (e[E_BUMP], array32(&e, E_PAYEE), array32(&e, E_COMMITMENT))
    };
    let held = read_token_amount(vault)?;
    let bump_seed = [bump];
    let seeds = [
        Seed::from(SEED_ESCROW),
        Seed::from(&stored_payee[..]),
        Seed::from(&commitment[..]),
        Seed::from(&bump_seed),
    ];
    let signer = [Signer::from(&seeds)];
    token_transfer(vault, destination, escrow, held, &signer)?;
    token_close(vault, payee, escrow, &signer)?;

    escrow.try_borrow_mut()?[E_DISC] = 0;
    payee.set_lamports(payee.lamports() + escrow.lamports());
    escrow.set_lamports(0);
    escrow.close()
}

/// Private Claim data: proof A (negated), B, C, then the Final Block Tree root the proof is against.
const PRIVATE_CLAIM_LEN: usize = 64 + 128 + 64 + 32;

/// Pays the Payer for a Groth16 proof that the committed transaction is in a final block inside the
/// Escrow's height window, under the same gates as a Reveal Claim: open or refund requested, Bond
/// deposited, relay running, root in the relay's recent history. The transaction ID, salt and block never
/// appear on Solana. The public inputs are the root the claimant names and three values the program reads
/// from its own state (commitment, lowest allowed height, expiry height), so a proof for a block outside
/// the window that a Reveal Claim accepts cannot verify.
fn private_claim(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [escrow, relay, vault, payout_token, payee, token_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if args.len() != PRIVATE_CLAIM_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut proof_a = [0u8; 64];
    proof_a.copy_from_slice(&args[..64]);
    let mut proof_b = [0u8; 128];
    proof_b.copy_from_slice(&args[64..192]);
    let mut proof_c = [0u8; 64];
    proof_c.copy_from_slice(&args[192..256]);
    let root = array32(args, 256);

    check_program(token_program, &TOKEN)?;
    check_escrow(program_id, escrow)?;
    let (commitment, created_tip, expiry_height, payout, mint) = {
        let e = escrow.try_borrow()?;
        if e[E_STATE] != STATE_OPEN && e[E_STATE] != STATE_REFUND_REQUESTED {
            return Err(fail(EscrowError::NotOpen));
        }
        if u64_at(&e, E_BOND_DEPOSITED) < u64_at(&e, E_BOND_REQUIRED) {
            return Err(fail(EscrowError::BondNotDeposited));
        }
        if array32(&e, E_RELAY) != *relay.address().as_ref()
            || array32(&e, E_VAULT) != *vault.address().as_ref()
            || array32(&e, E_PAYEE) != *payee.address().as_ref()
        {
            return Err(fail(EscrowError::WrongAccount));
        }
        (
            array32(&e, E_COMMITMENT),
            u32_at(&e, E_CREATED_TIP),
            u32_at(&e, E_EXPIRY_HEIGHT),
            array32(&e, E_PAYOUT),
            array32(&e, E_MINT),
        )
    };

    let lower = {
        let relay_data = relay.try_borrow()?;
        let view = relay_view(relay, &stored_relay(escrow)?, &relay_data)?;
        if !view.is_known_root(&root) {
            return Err(fail(EscrowError::UnknownRoot));
        }
        claim_window_lower(created_tip, view.finality_depth())
    };

    let mut public_inputs = [[0u8; 32]; verifying_key::PUBLIC_INPUTS];
    public_inputs[0] = root;
    public_inputs[1] = commitment;
    public_inputs[2][28..].copy_from_slice(&lower.to_be_bytes());
    public_inputs[3][28..].copy_from_slice(&expiry_height.to_be_bytes());
    verify_proof(&proof_a, &proof_b, &proof_c, &public_inputs)?;

    let paid = read_token(payout_token, false)?;
    if paid.mint != mint || paid.owner != payout {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    settle(escrow, vault, payout_token, payee)
}

/// An Escrow account of this program, writable, in the layout this version knows.
fn check_escrow(program_id: &Address, escrow: &AccountView) -> ProgramResult {
    if !escrow.owned_by(program_id) || !escrow.is_writable() {
        return Err(fail(EscrowError::WrongAccount));
    }
    let e = escrow.try_borrow()?;
    if e.len() != ESCROW_LEN || e[E_DISC] != DISC_ESCROW || e[E_VERSION] != VERSION {
        return Err(fail(EscrowError::WrongAccount));
    }
    Ok(())
}

fn deposit_bond(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [payer, escrow, payer_token, vault, token_program, relay] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !args.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    require_signer(payer)?;
    check_program(token_program, &TOKEN)?;
    check_escrow(program_id, escrow)?;
    let (bond, mint) = {
        let e = escrow.try_borrow()?;
        // Once a refund is requested a new Bond would only feed the Refund that may follow.
        if e[E_STATE] != STATE_OPEN {
            return Err(fail(EscrowError::NotOpen));
        }
        let bond = u64_at(&e, E_BOND_REQUIRED);
        if bond == 0 {
            return Err(fail(EscrowError::NoBondRequired));
        }
        if u64_at(&e, E_BOND_DEPOSITED) != 0 {
            return Err(fail(EscrowError::BondAlreadyDeposited));
        }
        // The Bond comes back to the payout wallet, so only that wallet may lock it.
        if array32(&e, E_PAYOUT) != *payer.address().as_ref() {
            return Err(fail(EscrowError::NotPayer));
        }
        if array32(&e, E_VAULT) != *vault.address().as_ref() {
            return Err(fail(EscrowError::WrongAccount));
        }
        (bond, array32(&e, E_MINT))
    };
    // Once the Payee could request a refund, a Bond could only feed it. A Halted relay fails here too:
    // no Claim can settle until it recovers or the backstop passes, so the Bond would sit idle.
    if refund_possible(escrow, relay, Clock::get()?.unix_timestamp)? {
        return Err(fail(EscrowError::BondTooLate));
    }
    let source = read_token(payer_token, false)?;
    if source.mint != mint || source.owner != *payer.address().as_ref() {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    token_transfer(payer_token, vault, payer, bond, &[])?;
    put(
        &mut escrow.try_borrow_mut()?,
        E_BOND_DEPOSITED,
        &bond.to_le_bytes(),
    );
    Ok(())
}

/// Whether a refund may be requested: 24 hours after creation whatever the relay says, or while the
/// Escrow's relay is running with a tip above the expiry height plus Finality Depth. A relay that is
/// not the Escrow's own, or is Halted before the backstop, is an error.
fn refund_possible(
    escrow: &AccountView,
    relay: &AccountView,
    now: i64,
) -> Result<bool, ProgramError> {
    let (created_at, expiry_height) = {
        let e = escrow.try_borrow()?;
        if array32(&e, E_RELAY) != *relay.address().as_ref() {
            return Err(fail(EscrowError::WrongAccount));
        }
        (i64_at(&e, E_CREATED_AT), u32_at(&e, E_EXPIRY_HEIGHT))
    };
    if now >= created_at.saturating_add(BACKSTOP_SECS) {
        return Ok(true);
    }
    let data = relay.try_borrow()?;
    let view = relay_view(relay, &stored_relay(escrow)?, &data)?;
    Ok(view.tip_height() > expiry_height.saturating_add(view.finality_depth() as u32))
}

/// Both refund steps need [`refund_possible`], so a Halt after the request sends the Payee back to
/// the backstop.
fn require_refundable(escrow: &AccountView, relay: &AccountView, now: i64) -> ProgramResult {
    if refund_possible(escrow, relay, now)? {
        Ok(())
    } else {
        Err(fail(EscrowError::RefundNotYet))
    }
}

/// First step of a refund, once [`require_refundable`] holds. Starts the claim window; a Claim still
/// settles until the Refund closes the Escrow.
fn request_refund(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [escrow, relay] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !args.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    check_escrow(program_id, escrow)?;
    if escrow.try_borrow()?[E_STATE] != STATE_OPEN {
        return Err(fail(EscrowError::NotOpen));
    }
    let now = Clock::get()?.unix_timestamp;
    require_refundable(escrow, relay, now)?;
    let mut e = escrow.try_borrow_mut()?;
    e[E_STATE] = STATE_REFUND_REQUESTED;
    put(&mut e, E_REFUND_REQUESTED_AT, &now.to_le_bytes());
    Ok(())
}

/// Second step: after the claim window the whole vault, Bond included, goes to the Payee.
fn refund(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [escrow, relay, vault, payee_token, payee, token_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !args.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    check_program(token_program, &TOKEN)?;
    check_escrow(program_id, escrow)?;
    let now = Clock::get()?.unix_timestamp;
    let mint = {
        let e = escrow.try_borrow()?;
        if e[E_STATE] != STATE_REFUND_REQUESTED {
            return Err(fail(EscrowError::RefundNotRequested));
        }
        let opens = i64_at(&e, E_REFUND_REQUESTED_AT).saturating_add(CLAIM_WINDOW_SECS);
        if now < opens {
            return Err(fail(EscrowError::ClaimWindowOpen));
        }
        if array32(&e, E_VAULT) != *vault.address().as_ref()
            || array32(&e, E_PAYEE) != *payee.address().as_ref()
        {
            return Err(fail(EscrowError::WrongAccount));
        }
        array32(&e, E_MINT)
    };
    require_refundable(escrow, relay, now)?;
    let back = read_token(payee_token, false)?;
    if back.mint != mint || back.owner != *payee.address().as_ref() {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    settle(escrow, vault, payee_token, payee)
}

fn read_token_amount(vault: &AccountView) -> Result<u64, ProgramError> {
    let d = vault.try_borrow()?;
    if d.len() != T_LEN {
        return Err(fail(EscrowError::BadTokenAccount));
    }
    Ok(u64_at(&d, T_AMOUNT))
}
