use pinocchio::{
    cpi::{invoke_signed, Seed, Signer},
    error::ProgramError,
    instruction::{InstructionAccount, InstructionView},
    sysvars::{rent::Rent, Sysvar},
    AccountView, Address, ProgramResult,
};
use zcash_header::{
    difficulty::{self, Rules, CONTEXT_LEN},
    equihash::{self, HEADER_LEN},
    header,
    pow::{self, Network},
};

use crate::{
    error::RelayError,
    finality::{self, Block, Cursor, Outcome},
    state::*,
    tree,
    zeros::{EMPTY_ROOT, ZEROS},
};

const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);

fn fail(e: RelayError) -> ProgramError {
    ProgramError::Custom(e as u32)
}

/// Solana syscalls. The program only ever runs on Solana; the host versions exist so
/// the processor type-checks and lints on a developer machine, and are never called.
#[cfg(target_os = "solana")]
mod sys {
    use pinocchio::Address;

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
    pub fn poseidon(inputs: &[&[u8; 32]]) -> Result<[u8; 32], pinocchio::error::ProgramError> {
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
            Err(pinocchio::error::ProgramError::Custom(
                crate::error::RelayError::PoseidonFailed as u32,
            ))
        }
    }
}

#[cfg(not(target_os = "solana"))]
mod sys {
    use pinocchio::Address;

    pub fn sha256(_: &[u8]) -> [u8; 32] {
        unreachable!("the Header Relay runs only on Solana")
    }

    pub fn find_program_address(_: &[&[u8]], _: &Address) -> (Address, u8) {
        unreachable!("the Header Relay runs only on Solana")
    }

    pub fn poseidon(_: &[&[u8; 32]]) -> Result<[u8; 32], pinocchio::error::ProgramError> {
        unreachable!("the Header Relay runs only on Solana")
    }
}

use sys::{find_program_address, poseidon, sha256};

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}
fn u32_at(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}
fn put(d: &mut [u8], at: usize, bytes: &[u8]) {
    d[at..at + bytes.len()].copy_from_slice(bytes);
}
fn array32(d: &[u8], at: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&d[at..at + 32]);
    out
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
        0 => create_relay(program_id, accounts, args),
        1 => write_checkpoint(program_id, accounts, args),
        2 => seal_relay(program_id, accounts),
        3 => grow_store(program_id, accounts),
        4 => create_buffer(program_id, accounts, args),
        5 => write_buffer(program_id, accounts, args),
        6 => begin_verify(program_id, accounts),
        7 => step_verify(program_id, accounts, args),
        8 => accept_header(program_id, accounts),
        9 => reset_buffer(program_id, accounts),
        10 => close_buffer(program_id, accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

/// Parses `offset u16, len u32, bytes` and returns the offset and the bytes.
fn offset_and_bytes(args: &[u8]) -> Result<(usize, &[u8]), ProgramError> {
    if args.len() < 6 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let offset = u16_at(args, 0) as usize;
    let len = u32_at(args, 2) as usize;
    let bytes = args[6..]
        .get(..len)
        .filter(|_| args.len() == 6 + len)
        .ok_or(ProgramError::InvalidInstructionData)?;
    Ok((offset, bytes))
}

/// Checks the account is a program-owned, correctly sized account of one kind.
fn check_kind(
    program_id: &Address,
    account: &AccountView,
    disc: u8,
    min_len: usize,
) -> ProgramResult {
    if !account.owned_by(program_id) || !account.is_writable() && disc != DISC_RELAY {
        return Err(fail(RelayError::WrongAccount));
    }
    let data = account.try_borrow()?;
    if data.len() < min_len || data[0] != disc || data[1] != VERSION {
        return Err(fail(RelayError::WrongAccount));
    }
    Ok(())
}

fn require_signer(account: &AccountView) -> ProgramResult {
    if account.is_signer() {
        Ok(())
    } else {
        Err(ProgramError::MissingRequiredSignature)
    }
}

fn check_system(account: &AccountView) -> ProgramResult {
    if account.address() == &SYSTEM_PROGRAM {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}

/// Creates a program-owned PDA account, also when someone already sent it lamports.
/// `N` is `seeds.len() + 1`: the seeds plus the bump.
fn create_pda<const N: usize>(
    program_id: &Address,
    payer: &AccountView,
    account: &AccountView,
    system: &AccountView,
    space: usize,
    seeds: &[&[u8]],
) -> ProgramResult {
    check_system(system)?;
    let (pda, bump) = find_program_address(seeds, program_id);
    if account.address() != &pda {
        return Err(fail(RelayError::WrongAccount));
    }
    let bump = [bump];
    let signer_seeds: [Seed; N] = core::array::from_fn(|i| {
        if i < seeds.len() {
            Seed::from(seeds[i])
        } else {
            Seed::from(&bump)
        }
    });
    let signer = Signer::from(&signer_seeds);
    let needed = Rent::get()?.try_minimum_balance(space)?;
    let have = account.lamports();
    if have == 0 {
        let mut data = [0u8; 52];
        data[4..12].copy_from_slice(&needed.to_le_bytes());
        data[12..20].copy_from_slice(&(space as u64).to_le_bytes());
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
            &[signer],
        )
    } else {
        if have < needed {
            transfer(payer, account, needed - have)?;
        }
        let mut allocate = [0u8; 12];
        allocate[..4].copy_from_slice(&8u32.to_le_bytes());
        allocate[4..].copy_from_slice(&(space as u64).to_le_bytes());
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
        )
    }
}

fn transfer(from: &AccountView, to: &AccountView, lamports: u64) -> ProgramResult {
    let mut data = [0u8; 12];
    data[..4].copy_from_slice(&2u32.to_le_bytes());
    data[4..].copy_from_slice(&lamports.to_le_bytes());
    invoke_signed(
        &InstructionView {
            program_id: &SYSTEM_PROGRAM,
            data: &data,
            accounts: &[
                InstructionAccount::writable_signer(from.address()),
                InstructionAccount::writable(to.address()),
            ],
        },
        &[from, to],
        &[],
    )
}

fn create_relay(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [payer, relay, store, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let digest: &[u8; 32] = args
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    require_signer(payer)?;
    create_pda::<4>(
        program_id,
        payer,
        relay,
        system,
        RELAY_LEN,
        &[SEED_RELAY, payer.address().as_ref(), digest],
    )?;
    create_pda::<3>(
        program_id,
        payer,
        store,
        system,
        INITIAL_STORE_LEN,
        &[SEED_STORE, relay.address().as_ref()],
    )?;
    let mut r = relay.try_borrow_mut()?;
    r[R_DISC] = DISC_RELAY;
    r[R_VERSION] = VERSION;
    r[R_STATE] = STATE_PENDING;
    put(&mut r, R_CREATOR, payer.address().as_ref());
    let mut s = store.try_borrow_mut()?;
    s[S_DISC] = DISC_STORE;
    s[S_VERSION] = VERSION;
    put(&mut s, S_RELAY, relay.address().as_ref());
    put(
        &mut s,
        S_CAPACITY,
        &(initial_capacity() as u16).to_le_bytes(),
    );
    Ok(())
}

fn write_checkpoint(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [creator, relay] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (offset, bytes) = offset_and_bytes(args)?;
    require_signer(creator)?;
    check_kind(program_id, relay, DISC_RELAY, RELAY_LEN)?;
    let mut r = relay.try_borrow_mut()?;
    if r[R_STATE] != STATE_PENDING {
        return Err(fail(RelayError::RelayAlreadySealed));
    }
    if r[R_CREATOR..R_CREATOR + 32] != *creator.address().as_ref() {
        return Err(ProgramError::IncorrectAuthority);
    }
    let end = offset
        .checked_add(bytes.len())
        .filter(|end| *end <= CHECKPOINT_LEN)
        .ok_or(ProgramError::InvalidInstructionData)?;
    r[R_CHECKPOINT + offset..R_CHECKPOINT + end].copy_from_slice(bytes);
    Ok(())
}

fn seal_relay(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [relay] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    check_kind(program_id, relay, DISC_RELAY, RELAY_LEN)?;
    let mut r = relay.try_borrow_mut()?;
    if r[R_STATE] != STATE_PENDING {
        return Err(fail(RelayError::RelayAlreadySealed));
    }
    let digest = sha256(&r[R_CHECKPOINT..R_CHECKPOINT + CHECKPOINT_LEN]);
    let creator = array32(&r, R_CREATOR);
    let (pda, _) = find_program_address(&[SEED_RELAY, &creator, &digest], program_id);
    if relay.address() != &pda {
        return Err(fail(RelayError::CheckpointDigestMismatch));
    }
    let cp = &r[R_CHECKPOINT..R_CHECKPOINT + CHECKPOINT_LEN];
    let network = cp[CP_NETWORK];
    let finality = u16_at(cp, CP_FINALITY_DEPTH);
    if cp[CP_VERSION] != VERSION
        || (network != NETWORK_MAINNET && network != NETWORK_TESTNET)
        || finality == 0
        || finality > MAX_FINALITY_DEPTH
        || cp[CP_HASH..CP_HASH + 32].iter().all(|b| *b == 0)
        || (0..CONTEXT_LEN).any(|i| pow::target_from_compact(u32_at(cp, CP_BITS + i * 4)).is_none())
    {
        return Err(fail(RelayError::InvalidCheckpoint));
    }
    let height = u32_at(cp, CP_HEIGHT);
    let hash = array32(cp, CP_HASH);
    r[R_STATE] = STATE_ACTIVE;
    put(&mut r, R_FINALIZED_HEIGHT, &height.to_le_bytes());
    put(&mut r, R_FINALIZED_HASH, &hash);
    for (level, zero) in ZEROS.iter().enumerate() {
        put(&mut r, R_FILLED + level * 32, zero);
    }
    put(&mut r, R_ROOTS, &EMPTY_ROOT);
    put(&mut r, R_TIP_HEIGHT, &height.to_le_bytes());
    put(&mut r, R_TIP_SLOT, &CHECKPOINT_SLOT.to_le_bytes());
    put(&mut r, R_TIP_HASH, &hash);
    Ok(())
}

fn grow_store(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [payer, store, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    require_signer(payer)?;
    check_system(system)?;
    check_kind(program_id, store, DISC_STORE, STORE_HEADER_LEN)?;
    let new_len = {
        let mut s = store.try_borrow_mut()?;
        let capacity = u16_at(&s, S_CAPACITY) as usize;
        let new_capacity = capacity + GROW_BYTES / SLOT_LEN;
        if new_capacity > MAX_CAPACITY {
            return Err(fail(RelayError::StoreNotGrowable));
        }
        put(&mut s, S_CAPACITY, &(new_capacity as u16).to_le_bytes());
        STORE_HEADER_LEN + new_capacity * SLOT_LEN
    };
    store.resize(new_len)?;
    let needed = Rent::get()?.try_minimum_balance(new_len)?;
    if store.lamports() < needed {
        transfer(payer, store, needed - store.lamports())?;
    }
    Ok(())
}

fn create_buffer(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [authority, relay, buffer, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let [nonce] = args else {
        return Err(ProgramError::InvalidInstructionData);
    };
    require_signer(authority)?;
    check_kind(program_id, relay, DISC_RELAY, RELAY_LEN)?;
    {
        let r = relay.try_borrow()?;
        if r[R_STATE] != STATE_ACTIVE {
            return Err(fail(RelayError::RelayNotActive));
        }
        if r[R_HALTED] != HALT_RUNNING {
            return Err(fail(RelayError::RelayHalted));
        }
    }
    create_pda::<5>(
        program_id,
        authority,
        buffer,
        system,
        BUFFER_LEN,
        &[
            SEED_BUFFER,
            relay.address().as_ref(),
            authority.address().as_ref(),
            &[*nonce],
        ],
    )?;
    let mut b = buffer.try_borrow_mut()?;
    b[B_DISC] = DISC_BUFFER;
    b[B_VERSION] = VERSION;
    put(&mut b, B_RELAY, relay.address().as_ref());
    put(&mut b, B_AUTHORITY, authority.address().as_ref());
    Ok(())
}

/// Validates a buffer and that `authority` signed and owns it.
fn check_buffer(
    program_id: &Address,
    authority: &AccountView,
    buffer: &AccountView,
) -> ProgramResult {
    require_signer(authority)?;
    check_kind(program_id, buffer, DISC_BUFFER, BUFFER_LEN)?;
    if buffer.try_borrow()?[B_AUTHORITY..B_AUTHORITY + 32] != *authority.address().as_ref() {
        return Err(ProgramError::IncorrectAuthority);
    }
    Ok(())
}

fn write_buffer(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [authority, buffer] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let (offset, bytes) = offset_and_bytes(args)?;
    check_buffer(program_id, authority, buffer)?;
    let mut b = buffer.try_borrow_mut()?;
    if !equihash::is_clear(&b[B_VERIFIER..]) || b[B_ACCEPTED] != 0 {
        return Err(fail(RelayError::HeaderFrozen));
    }
    let end = offset
        .checked_add(bytes.len())
        .filter(|end| *end <= HEADER_LEN)
        .ok_or(ProgramError::InvalidInstructionData)?;
    b[B_HEADER + offset..B_HEADER + end].copy_from_slice(bytes);
    Ok(())
}

fn begin_verify(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [authority, buffer] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    check_buffer(program_id, authority, buffer)?;
    let mut b = buffer.try_borrow_mut()?;
    if b[B_ACCEPTED] != 0 {
        return Err(fail(RelayError::AlreadyAccepted));
    }
    let (front, verifier) = b.split_at_mut(B_VERIFIER);
    equihash::begin(&front[B_HEADER..], verifier).map_err(|e| fail(e.into()))
}

fn step_verify(program_id: &Address, accounts: &[AccountView], args: &[u8]) -> ProgramResult {
    let [authority, buffer] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let leaves: [u8; 2] = args
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    check_buffer(program_id, authority, buffer)?;
    let mut b = buffer.try_borrow_mut()?;
    if b[B_ACCEPTED] != 0 {
        return Err(fail(RelayError::AlreadyAccepted));
    }
    let (front, verifier) = b.split_at_mut(B_VERIFIER);
    equihash::step(&front[B_HEADER..], verifier, u16::from_le_bytes(leaves))
        .map(|_| ())
        .map_err(|e| fail(e.into()))
}

fn reset_buffer(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [authority, buffer] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    check_buffer(program_id, authority, buffer)?;
    let mut b = buffer.try_borrow_mut()?;
    if b[B_ACCEPTED] != 0 {
        return Err(fail(RelayError::AlreadyAccepted));
    }
    b[B_VERIFIER..].fill(0);
    Ok(())
}

fn close_buffer(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [authority, buffer] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    check_buffer(program_id, authority, buffer)?;
    authority.set_lamports(authority.lamports() + buffer.lamports());
    buffer.set_lamports(0);
    buffer.close()
}

/// Times and `nBits` of the blocks before a candidate, newest first: `[0]` is its parent.
struct Context {
    times: [u32; CONTEXT_LEN],
    bits: [u32; CONTEXT_LEN],
}

/// Reads the `len` blocks that end at `parent` by following parent pointers through the Store and,
/// past the first header, through the Checkpoint's 113 blocks. A pointer is valid only while its
/// slot still holds the block it was made for; an evicted ancestor is `ContextUnavailable`.
#[inline(never)]
fn load_context(
    r: &[u8],
    s: &[u8],
    capacity: usize,
    parent: &Parent,
    len: usize,
) -> Result<Context, ProgramError> {
    let mut ctx = Context {
        times: [0; CONTEXT_LEN],
        bits: [0; CONTEXT_LEN],
    };
    let (mut slot, mut seq) = (parent.slot, parent.seq);
    for i in 0..len {
        if slot == CHECKPOINT_SLOT {
            let from_checkpoint = i;
            for j in i..len {
                let back = (CONTEXT_LEN - 1) - (j - from_checkpoint);
                ctx.times[j] = u32_at(r, R_CHECKPOINT + CP_TIMES + back * 4);
                ctx.bits[j] = u32_at(r, R_CHECKPOINT + CP_BITS + back * 4);
            }
            return Ok(ctx);
        }
        let index = slot as usize;
        if index >= capacity {
            return Err(fail(RelayError::ContextUnavailable));
        }
        let at = STORE_HEADER_LEN + index * SLOT_LEN;
        let entry = &s[at..at + SLOT_LEN];
        if seq == 0 || u32_at(entry, SL_SEQ) != seq {
            return Err(fail(RelayError::ContextUnavailable));
        }
        ctx.times[i] = u32_at(entry, SL_TIME);
        ctx.bits[i] = u32_at(entry, SL_BITS);
        slot = u16_at(entry, SL_PARENT_SLOT);
        seq = u32_at(entry, SL_PARENT_SEQ);
    }
    Ok(ctx)
}

/// Where a candidate's parent sits.
struct Parent {
    slot: u16,
    seq: u32,
    height: u32,
    work: u128,
}

fn slot_range(index: usize) -> core::ops::Range<usize> {
    let at = STORE_HEADER_LEN + index * SLOT_LEN;
    at..at + SLOT_LEN
}

fn u128_at(d: &[u8], at: usize) -> u128 {
    let mut w = [0u8; 16];
    w.copy_from_slice(&d[at..at + 16]);
    u128::from_le_bytes(w)
}

/// Checks the candidate's nTime and nBits against the difficulty context of its parent chain.
#[inline(never)]
fn check_difficulty(
    r: &[u8],
    s: &[u8],
    capacity: usize,
    parent: &Parent,
    rules: &Rules,
    height: u32,
    header_bytes: &[u8; HEADER_LEN],
) -> ProgramResult {
    let ctx = load_context(
        r,
        s,
        capacity,
        parent,
        rules.window(height) + difficulty::MEDIAN_SPAN,
    )?;
    let time = header::time(header_bytes);
    difficulty::check_time(rules, height, time, &ctx.times).map_err(|e| fail(e.into()))?;
    let expected = difficulty::expected_bits(rules, height, time, &ctx.times, &ctx.bits)
        .map_err(|e| fail(e.into()))?;
    if header::bits(header_bytes) != expected {
        return Err(fail(RelayError::BadDifficulty));
    }
    Ok(())
}

/// What a new tip does to the Final Block Tree.
enum Step {
    Append {
        slots: [u16; finality::MAX_APPENDS],
        count: usize,
    },
    Halt(u8),
}

/// Asks which blocks the new tip finalises. An error here leaves the relay untouched.
#[inline(never)]
fn plan_finality(
    r: &[u8],
    s: &[u8],
    capacity: usize,
    height: u32,
    parent: &Parent,
) -> Result<Step, ProgramError> {
    let cursor = Cursor {
        checkpoint_height: u32_at(r, R_CHECKPOINT + CP_HEIGHT),
        checkpoint_hash: array32(r, R_CHECKPOINT + CP_HASH),
        height: u32_at(r, R_FINALIZED_HEIGHT),
        hash: array32(r, R_FINALIZED_HASH),
    };
    let depth = u16_at(r, R_CHECKPOINT + CP_FINALITY_DEPTH);
    let outcome = finality::plan(
        &cursor,
        depth,
        height,
        parent.slot,
        parent.seq,
        |slot, seq| {
            let index = slot as usize;
            if index >= capacity || seq == 0 {
                return None;
            }
            let entry = &s[slot_range(index)];
            (u32_at(entry, SL_SEQ) == seq).then(|| Block {
                height: u32_at(entry, SL_HEIGHT),
                hash: array32(entry, SL_HASH),
                parent_slot: u16_at(entry, SL_PARENT_SLOT),
                parent_seq: u32_at(entry, SL_PARENT_SEQ),
            })
        },
    );
    match outcome {
        Outcome::Reorg => Ok(Step::Halt(HALT_DEEP_REORG)),
        Outcome::Unreadable => Err(fail(RelayError::ContextUnavailable)),
        Outcome::TooManyAppends => Err(fail(RelayError::TooManyAppends)),
        Outcome::Extend { slots, count } => {
            if u32_at(r, R_LEAF_COUNT) as u64 + count as u64 > TREE_CAPACITY {
                Ok(Step::Halt(HALT_TREE_FULL))
            } else {
                Ok(Step::Append { slots, count })
            }
        }
    }
}

/// Appends the blocks in `slots` (oldest first) to the Final Block Tree and moves the cursor.
#[inline(never)]
fn append_final_blocks(r: &mut [u8], s: &[u8], slots: &[u16]) -> ProgramResult {
    if slots.is_empty() {
        return Ok(());
    }
    let mut filled = [[0u8; 32]; TREE_DEPTH];
    for (level, node) in filled.iter_mut().enumerate() {
        *node = array32(r, R_FILLED + level * 32);
    }
    let mut count = u32_at(r, R_LEAF_COUNT);
    let (mut last_height, mut last_hash) = (0, [0u8; 32]);
    for slot in slots {
        let entry = &s[slot_range(*slot as usize)];
        last_height = u32_at(entry, SL_HEIGHT);
        last_hash = array32(entry, SL_HASH);
        let [h, hi, lo] = leaf_inputs(last_height, &array32(entry, SL_MERKLE_ROOT));
        let leaf = poseidon(&[&h, &hi, &lo])?;
        let root = tree::append(&mut filled, &ZEROS, count as u64, leaf, |a, b| {
            poseidon(&[a, b])
        })?;
        count += 1;
        put(r, R_ROOTS + (count as usize % ROOT_HISTORY) * 32, &root);
    }
    for (level, node) in filled.iter().enumerate() {
        put(r, R_FILLED + level * 32, node);
    }
    put(r, R_LEAF_COUNT, &count.to_le_bytes());
    put(r, R_FINALIZED_HEIGHT, &last_height.to_le_bytes());
    put(r, R_FINALIZED_HASH, &last_hash);
    Ok(())
}

/// Chooses the slot for a new header, never one of the protected chains, and applies the admission rule.
#[inline(never)]
fn choose_victim(
    r: &[u8],
    s: &[u8],
    capacity: usize,
    parent: &Parent,
    class: u16,
    becomes_tip: bool,
    new_height: u32,
) -> Result<usize, ProgramError> {
    let link = |i: usize| {
        let entry = &s[slot_range(i)];
        (
            u32_at(entry, SL_SEQ),
            u16_at(entry, SL_PARENT_SLOT),
            u32_at(entry, SL_PARENT_SEQ),
        )
    };
    // The chain the relay will follow after this header is what must stay readable: the new parent's
    // chain when the header becomes the tip, otherwise the current tip's chain plus the parent's
    // difficulty context (which is a competing branch above the fork point).
    let mut protected = [false; MAX_CAPACITY];
    let tip_depth = protected_tip_depth(u16_at(r, R_CHECKPOINT + CP_FINALITY_DEPTH));
    if becomes_tip {
        mark_chain(parent.slot, parent.seq, tip_depth, link, &mut protected);
    } else {
        mark_chain(
            u16_at(r, R_TIP_SLOT),
            u32_at(r, R_TIP_SEQ),
            tip_depth,
            link,
            &mut protected,
        );
        mark_chain(parent.slot, parent.seq, CONTEXT_LEN, link, &mut protected);
    }
    let tip_height = if becomes_tip {
        new_height
    } else {
        u32_at(r, R_TIP_HEIGHT)
    };
    let victim = choose_slot(
        capacity,
        |i| {
            let entry = &s[slot_range(i)];
            SlotInfo {
                seq: u32_at(entry, SL_SEQ),
                stale: is_too_old(tip_height, u32_at(entry, SL_HEIGHT) + 1),
                class: difficulty_class(u32_at(entry, SL_BITS)),
            }
        },
        |i| protected[i],
    )
    .ok_or(fail(RelayError::StoreFull))?;
    if !admits(&victim, class, becomes_tip) {
        return Err(fail(RelayError::NotAdmitted));
    }
    Ok(victim.index)
}

/// Whether a stored header's parent pointer still leads to its parent.
fn link_is_valid(r: &[u8], s: &[u8], capacity: usize, entry: &[u8], prev: &[u8; 32]) -> bool {
    let (slot, seq) = (u16_at(entry, SL_PARENT_SLOT), u32_at(entry, SL_PARENT_SEQ));
    if slot == CHECKPOINT_SLOT {
        return *prev == array32(r, R_CHECKPOINT + CP_HASH);
    }
    let index = slot as usize;
    if index >= capacity || seq == 0 {
        return false;
    }
    let parent = &s[slot_range(index)];
    u32_at(parent, SL_SEQ) == seq && parent[SL_HASH..SL_HASH + 32] == *prev
}

fn accept_header(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [authority, buffer, relay, store] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    check_buffer(program_id, authority, buffer)?;
    check_kind(program_id, relay, DISC_RELAY, RELAY_LEN)?;
    check_kind(program_id, store, DISC_STORE, STORE_HEADER_LEN)?;
    if !relay.is_writable() {
        return Err(fail(RelayError::WrongAccount));
    }
    let mut b = buffer.try_borrow_mut()?;
    let mut r = relay.try_borrow_mut()?;
    let mut s = store.try_borrow_mut()?;
    if b[B_RELAY..B_RELAY + 32] != *relay.address().as_ref()
        || s[S_RELAY..S_RELAY + 32] != *relay.address().as_ref()
    {
        return Err(fail(RelayError::WrongAccount));
    }
    if r[R_STATE] != STATE_ACTIVE {
        return Err(fail(RelayError::RelayNotActive));
    }
    if r[R_HALTED] != HALT_RUNNING {
        return Err(fail(RelayError::RelayHalted));
    }
    if b[B_ACCEPTED] != 0 {
        return Err(fail(RelayError::AlreadyAccepted));
    }
    if !equihash::is_verified(&b[B_VERIFIER..]) {
        return Err(fail(RelayError::NotVerified));
    }
    let header_bytes: &[u8; HEADER_LEN] = b[B_HEADER..B_HEADER + HEADER_LEN]
        .try_into()
        .map_err(|_| fail(RelayError::MalformedHeader))?;

    let network = if r[R_CHECKPOINT + CP_NETWORK] == NETWORK_MAINNET {
        Network::Mainnet
    } else {
        Network::Testnet
    };
    if header::version(header_bytes) < header::MIN_VERSION {
        return Err(fail(RelayError::InvalidVersion));
    }
    let hash = header::hash(sha256, header_bytes);
    let target =
        pow::check_pow(network, header::bits(header_bytes), &hash).map_err(|e| fail(e.into()))?;
    let work = pow::work(&target).ok_or(fail(RelayError::WorkOverflow))?;

    let capacity = u16_at(&s, S_CAPACITY) as usize;
    if capacity < MIN_CAPACITY {
        return Err(fail(RelayError::StoreTooSmall));
    }
    let count = u32_at(&s, S_COUNT);
    let prev = header::prev_hash(header_bytes);
    let checkpoint_hash = array32(&r, R_CHECKPOINT + CP_HASH);
    if hash == checkpoint_hash {
        return Err(fail(RelayError::DuplicateHeader));
    }
    let mut parent = (prev == checkpoint_hash).then(|| Parent {
        slot: CHECKPOINT_SLOT,
        seq: 0,
        height: u32_at(&r, R_CHECKPOINT + CP_HEIGHT),
        work: 0,
    });
    let mut duplicate = None;
    for i in 0..capacity {
        let slot = &s[slot_range(i)];
        if u32_at(slot, SL_SEQ) == 0 {
            continue;
        }
        if slot[SL_HASH..SL_HASH + 32] == hash {
            duplicate = Some(i);
            continue;
        }
        if slot[SL_HASH..SL_HASH + 32] == prev {
            parent = Some(Parent {
                slot: i as u16,
                seq: u32_at(slot, SL_SEQ),
                height: u32_at(slot, SL_HEIGHT),
                work: u128_at(slot, SL_WORK),
            });
        }
    }
    if let Some(i) = duplicate {
        // A stored header whose parent was evicted and later re-submitted points at a dead slot:
        // submitting it again repairs the pointer. Otherwise it is a plain duplicate.
        if link_is_valid(&r, &s, capacity, &s[slot_range(i)], &prev) {
            return Err(fail(RelayError::DuplicateHeader));
        }
        let parent = parent.ok_or(fail(RelayError::UnknownParent))?;
        let at = slot_range(i).start;
        put(&mut s, at + SL_PARENT_SLOT, &parent.slot.to_le_bytes());
        put(&mut s, at + SL_PARENT_SEQ, &parent.seq.to_le_bytes());
        b[B_ACCEPTED] = 1;
        return Ok(());
    }
    let parent = parent.ok_or(fail(RelayError::UnknownParent))?;

    let height = parent.height + 1;
    let tip_height = u32_at(&r, R_TIP_HEIGHT);
    if is_too_old(tip_height, height) {
        return Err(fail(RelayError::TooOld));
    }
    let rules = Rules {
        network,
        nu7_height: u32_at(&r, R_CHECKPOINT + CP_NU7_HEIGHT),
    };
    check_difficulty(&r, &s, capacity, &parent, &rules, height, header_bytes)?;

    let total = parent
        .work
        .checked_add(work)
        .ok_or(fail(RelayError::WorkOverflow))?;
    let becomes_tip = total > u128_at(&r, R_TIP_WORK);
    let mut appends = ([0u16; finality::MAX_APPENDS], 0);
    if becomes_tip {
        match plan_finality(&r, &s, capacity, height, &parent)? {
            Step::Halt(reason) => {
                r[R_HALTED] = reason;
                put(&mut r, R_HALT_HEIGHT, &tip_height.to_le_bytes());
                put(&mut r, R_HALT_HASH, &hash);
                b[B_ACCEPTED] = 1;
                return Ok(());
            }
            Step::Append { slots, count } => appends = (slots, count),
        }
    }
    let index = choose_victim(
        &r,
        &s,
        capacity,
        &parent,
        difficulty_class(header::bits(header_bytes)),
        becomes_tip,
        height,
    )?;
    let seq = count
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    append_final_blocks(&mut r, &s, &appends.0[..appends.1])?;

    let at = slot_range(index).start;
    let slot = &mut s[at..at + SLOT_LEN];
    put(slot, SL_HASH, &hash);
    put(slot, SL_MERKLE_ROOT, &header::merkle_root(header_bytes));
    put(slot, SL_WORK, &total.to_le_bytes());
    put(slot, SL_HEIGHT, &height.to_le_bytes());
    put(slot, SL_TIME, &header::time(header_bytes).to_le_bytes());
    put(slot, SL_BITS, &header::bits(header_bytes).to_le_bytes());
    put(slot, SL_SEQ, &seq.to_le_bytes());
    put(slot, SL_PARENT_SLOT, &parent.slot.to_le_bytes());
    put(slot, SL_PARENT_SEQ, &parent.seq.to_le_bytes());
    put(&mut s, S_COUNT, &seq.to_le_bytes());

    if becomes_tip {
        put(&mut r, R_TIP_HEIGHT, &height.to_le_bytes());
        put(&mut r, R_TIP_SLOT, &(index as u16).to_le_bytes());
        put(&mut r, R_TIP_SEQ, &seq.to_le_bytes());
        put(&mut r, R_TIP_WORK, &total.to_le_bytes());
        put(&mut r, R_TIP_HASH, &hash);
    }
    b[B_ACCEPTED] = 1;
    Ok(())
}
