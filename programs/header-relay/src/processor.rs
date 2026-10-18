use pinocchio::{
    cpi::{invoke_signed, Seed, Signer},
    error::ProgramError,
    instruction::{InstructionAccount, InstructionView},
    sysvars::{rent::Rent, Sysvar},
    AccountView, Address, ProgramResult,
};
use zcash_header::{
    equihash::{self, HEADER_LEN},
    header,
    pow::{self, Network},
};

use crate::{error::RelayError, state::*};

const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);

fn fail(e: RelayError) -> ProgramError {
    ProgramError::Custom(e as u32)
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let vals: [&[u8]; 1] = [data];
    // SAFETY: `vals` is a one-element slice-of-slices, the layout the syscall reads.
    unsafe {
        pinocchio::syscalls::sol_sha256(vals.as_ptr() as *const u8, 1, out.as_mut_ptr());
    }
    out
}

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
    let (pda, bump) = Address::find_program_address(seeds, program_id);
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
    create_pda::<3>(
        program_id,
        payer,
        relay,
        system,
        RELAY_LEN,
        &[SEED_RELAY, digest],
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
    put(&mut s, S_CAPACITY, &(initial_capacity() as u16).to_le_bytes());
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
    let (pda, _) = Address::find_program_address(&[SEED_RELAY, &digest], program_id);
    if relay.address() != &pda {
        return Err(fail(RelayError::CheckpointDigestMismatch));
    }
    let cp = &r[R_CHECKPOINT..R_CHECKPOINT + CHECKPOINT_LEN];
    let network = cp[CP_NETWORK];
    let finality = u16_at(cp, CP_FINALITY_DEPTH);
    if cp[CP_VERSION] != VERSION
        || (network != NETWORK_MAINNET && network != NETWORK_TESTNET)
        || finality == 0
        || cp[CP_HASH..CP_HASH + 32].iter().all(|b| *b == 0)
    {
        return Err(fail(RelayError::InvalidCheckpoint));
    }
    let height = u32_at(cp, CP_HEIGHT);
    let hash = array32(cp, CP_HASH);
    r[R_STATE] = STATE_ACTIVE;
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
        let count = u32_at(&s, S_COUNT) as usize;
        let new_capacity = capacity + GROW_BYTES / SLOT_LEN;
        if count > capacity || new_capacity > MAX_CAPACITY {
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
    if relay.try_borrow()?[R_STATE] != STATE_ACTIVE {
        return Err(fail(RelayError::RelayNotActive));
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

/// Where a candidate's parent sits.
struct Parent {
    slot: u16,
    seq: u32,
    height: u32,
    work: u128,
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
    if r[R_HALTED] != 0 {
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
    let hash = header::hash(sha256, header_bytes);
    let target = pow::check_pow(network, header::bits(header_bytes), &hash)
        .map_err(|e| fail(e.into()))?;
    let work = pow::work(&target);

    let capacity = u16_at(&s, S_CAPACITY) as usize;
    let count = u32_at(&s, S_COUNT);
    let stored = (count as usize).min(capacity);
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
    for i in 0..stored {
        let at = STORE_HEADER_LEN + i * SLOT_LEN;
        let slot = &s[at..at + SLOT_LEN];
        if slot[SL_HASH..SL_HASH + 32] == hash {
            return Err(fail(RelayError::DuplicateHeader));
        }
        if slot[SL_HASH..SL_HASH + 32] == prev {
            let mut w = [0u8; 16];
            w.copy_from_slice(&slot[SL_WORK..SL_WORK + 16]);
            parent = Some(Parent {
                slot: i as u16,
                seq: u32_at(slot, SL_SEQ),
                height: u32_at(slot, SL_HEIGHT),
                work: u128::from_le_bytes(w),
            });
        }
    }
    let parent = parent.ok_or(fail(RelayError::UnknownParent))?;

    let index = count as usize % capacity;
    if u16_at(&r, R_TIP_SLOT) as usize == index && count as usize >= capacity {
        return Err(fail(RelayError::StoreFull));
    }
    let seq = count + 1;
    let height = parent.height + 1;
    let total = parent.work.saturating_add(work);
    let at = STORE_HEADER_LEN + index * SLOT_LEN;
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

    let mut tip_work = [0u8; 16];
    tip_work.copy_from_slice(&r[R_TIP_WORK..R_TIP_WORK + 16]);
    if total > u128::from_le_bytes(tip_work) {
        put(&mut r, R_TIP_HEIGHT, &height.to_le_bytes());
        put(&mut r, R_TIP_SLOT, &(index as u16).to_le_bytes());
        put(&mut r, R_TIP_SEQ, &seq.to_le_bytes());
        put(&mut r, R_TIP_WORK, &total.to_le_bytes());
        put(&mut r, R_TIP_HASH, &hash);
    }
    b[B_ACCEPTED] = 1;
    Ok(())
}
