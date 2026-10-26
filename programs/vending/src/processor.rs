use crate::{
    error::VendingError as E,
    rules::{can_release, validate_claim, view_error, ClaimAccounts},
    state::*,
};
use header_relay_interface::{RelayStatus, RelayView, NETWORK_TESTNET};
use pinocchio::{
    cpi::{invoke_signed, Seed, Signer},
    error::ProgramError,
    instruction::{InstructionAccount as Meta, InstructionView},
    sysvars::{clock::Clock, rent::Rent, Sysvar},
    AccountView, Address, ProgramResult,
};
use vending_types::{PublicValues, PUBLIC_VALUES_LEN};

const SYSTEM: Address = Address::new_from_array([0; 32]);
const TOKEN: Address = Address::new_from_array(TOKEN_PROGRAM);
// The associated-token program's canonical bytes, pinned independently of the supplied accounts.
const ATA: Address = Address::new_from_array([
    140, 151, 37, 143, 78, 36, 137, 241, 187, 61, 16, 41, 20, 142, 13, 131, 11, 90, 19, 153, 218,
    255, 16, 132, 4, 142, 123, 216, 219, 233, 248, 89,
]);
fn fail(e: E) -> ProgramError {
    ProgramError::Custom(e as u32)
}
#[cfg(target_os = "solana")]
fn pda(seeds: &[&[u8]], program: &Address) -> (Address, u8) {
    Address::find_program_address(seeds, program)
}
#[cfg(not(target_os = "solana"))]
fn pda(_: &[&[u8]], _: &Address) -> (Address, u8) {
    unreachable!("instruction processor runs in the SVM")
}
fn signer(a: &AccountView) -> ProgramResult {
    if a.is_signer() {
        Ok(())
    } else {
        Err(ProgramError::MissingRequiredSignature)
    }
}
fn program(a: &AccountView, id: &Address) -> ProgramResult {
    if a.address() == id {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}
fn writable(a: &AccountView) -> ProgramResult {
    if a.is_writable() {
        Ok(())
    } else {
        Err(ProgramError::InvalidAccountData)
    }
}
fn expect_key(a: &AccountView, key: &[u8; 32]) -> ProgramResult {
    if a.address().as_array() == key {
        Ok(())
    } else {
        Err(fail(E::WrongAccount))
    }
}
fn owner(a: &AccountView) -> [u8; 32] {
    // SAFETY: copied before any CPI; no account data reference escapes.
    unsafe { *a.owner().as_array() }
}
fn machine(id: &Address, a: &AccountView) -> Result<Machine, ProgramError> {
    if !a.owned_by(id) {
        return Err(fail(E::WrongAccount));
    }
    let m = Machine::read(&a.try_borrow()?).ok_or(fail(E::WrongAccount))?;
    if pda(&[b"machine", &m.payee, &m.seed], id) != (*a.address(), m.bump) || m.available > m.stock
    {
        return Err(fail(E::WrongAccount));
    }
    Ok(m)
}
fn reservation(id: &Address, a: &AccountView, key: &[u8; 32]) -> Result<Reservation, ProgramError> {
    if !a.owned_by(id) {
        return Err(fail(E::MissingReservation));
    }
    let r = Reservation::read(&a.try_borrow()?).ok_or(fail(E::MissingReservation))?;
    if &r.machine != key || pda(&[b"reservation", key, &r.nonce], id) != (*a.address(), r.bump) {
        return Err(fail(E::WrongAccount));
    }
    Ok(r)
}
fn relay<'a>(
    a: &AccountView,
    expected: &[u8; 32],
    d: &'a [u8],
) -> Result<RelayView<'a>, ProgramError> {
    RelayView::parse(&owner(a), a.address().as_array(), expected, d)
        .map_err(|e| fail(view_error(e)))
}
/// Creates a new account, including system-owned empty accounts prefunded with lamports.
fn create(
    payer: &AccountView,
    a: &AccountView,
    id: &Address,
    len: usize,
    seeds: &[Seed<'_>],
) -> ProgramResult {
    writable(a)?;
    if !a.owned_by(&SYSTEM) || !a.is_data_empty() {
        return Err(fail(E::AlreadyInitialized));
    }
    let needed = Rent::get()?.try_minimum_balance(len)?;
    let signers = [Signer::from(seeds)];
    if a.lamports() == 0 {
        let mut d = [0; 52];
        d[4..12].copy_from_slice(&needed.to_le_bytes());
        d[12..20].copy_from_slice(&(len as u64).to_le_bytes());
        d[20..52].copy_from_slice(id.as_ref());
        invoke_signed(
            &InstructionView {
                program_id: &SYSTEM,
                data: &d,
                accounts: &[
                    Meta::writable_signer(payer.address()),
                    Meta::writable_signer(a.address()),
                ],
            },
            &[payer, a],
            if seeds.is_empty() { &[] } else { &signers },
        )?;
    } else {
        if a.lamports() < needed {
            let mut d = [0; 12];
            d[..4].copy_from_slice(&2u32.to_le_bytes());
            d[4..].copy_from_slice(&(needed - a.lamports()).to_le_bytes());
            invoke_signed(
                &InstructionView {
                    program_id: &SYSTEM,
                    data: &d,
                    accounts: &[
                        Meta::writable_signer(payer.address()),
                        Meta::writable(a.address()),
                    ],
                },
                &[payer, a],
                &[],
            )?;
        }
        let mut d = [0; 12];
        d[..4].copy_from_slice(&8u32.to_le_bytes());
        d[4..].copy_from_slice(&(len as u64).to_le_bytes());
        invoke_signed(
            &InstructionView {
                program_id: &SYSTEM,
                data: &d,
                accounts: &[Meta::writable_signer(a.address())],
            },
            &[a],
            if seeds.is_empty() { &[] } else { &signers },
        )?;
        let mut d = [0; 36];
        d[..4].copy_from_slice(&1u32.to_le_bytes());
        d[4..].copy_from_slice(id.as_ref());
        invoke_signed(
            &InstructionView {
                program_id: &SYSTEM,
                data: &d,
                accounts: &[Meta::writable_signer(a.address())],
            },
            &[a],
            if seeds.is_empty() { &[] } else { &signers },
        )?;
    }
    Ok(())
}
fn new_pda(
    payer: &AccountView,
    a: &AccountView,
    id: &Address,
    len: usize,
    prefix: &[u8],
    one: &[u8; 32],
    two: &[u8; 32],
) -> Result<u8, ProgramError> {
    let (key, bump) = pda(&[prefix, one, two], id);
    expect_key(a, key.as_array())?;
    create(
        payer,
        a,
        id,
        len,
        &[
            Seed::from(prefix),
            Seed::from(one),
            Seed::from(two),
            Seed::from(&[bump]),
        ],
    )?;
    Ok(bump)
}
fn close(a: &AccountView, payout: &AccountView) -> ProgramResult {
    writable(a)?;
    writable(payout)?;
    let balance = payout
        .lamports()
        .checked_add(a.lamports())
        .ok_or(fail(E::ArithmeticOverflow))?;
    a.try_borrow_mut()?.fill(0);
    payout.set_lamports(balance);
    a.set_lamports(0);
    a.close()
}
fn arr<const N: usize>(d: &[u8], at: usize) -> [u8; N] {
    d[at..at + N]
        .try_into()
        .expect("validated instruction length")
}

pub fn process_instruction(id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let (&tag, args) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match tag {
        0 => init(id, accounts, args),
        1 => reserve(id, accounts, args),
        2 => claim(id, accounts, args),
        3 => release(id, accounts, args),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
fn init(id: &Address, accounts: &[AccountView], d: &[u8]) -> ProgramResult {
    let [payee, machine_account, mint, authority, relay_account, token, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if d.len() != 155 {
        return Err(ProgramError::InvalidInstructionData);
    }
    signer(payee)?;
    signer(mint)?;
    writable(payee)?;
    program(token, &TOKEN)?;
    program(system, &SYSTEM)?;
    let seed = arr(d, 0);
    let price_zat = u64::from_le_bytes(arr(d, 32));
    let stock = u32::from_le_bytes(arr(d, 40));
    let reservation_blocks = u32::from_le_bytes(arr(d, 151));
    if price_zat == 0 || stock == 0 || reservation_blocks == 0 {
        return Err(fail(E::BadConfig));
    }
    let expected_relay = arr(d, 87);
    let rd = relay_account.try_borrow()?;
    let view = relay(relay_account, &expected_relay, &rd)?;
    if view.network() != NETWORK_TESTNET {
        return Err(fail(E::WrongBranch));
    }
    drop(rd);
    let (auth, mint_bump) = pda(&[b"mint", machine_account.address().as_ref()], id);
    expect_key(authority, auth.as_array())?;
    let bump = new_pda(
        payee,
        machine_account,
        id,
        MACHINE_LEN,
        b"machine",
        payee.address().as_array(),
        &seed,
    )?;
    create(payee, mint, &TOKEN, 82, &[])?;
    // InitializeMint2: no freeze authority; decimals zero. SPL's COption here is one byte.
    let mut initialize = [0; 35];
    initialize[0] = 20;
    initialize[2..34].copy_from_slice(auth.as_ref());
    invoke_signed(
        &InstructionView {
            program_id: &TOKEN,
            data: &initialize,
            accounts: &[Meta::writable(mint.address())],
        },
        &[mint],
        &[],
    )?;
    Machine {
        bump,
        mint_bump,
        payee: *payee.address().as_array(),
        seed,
        mint: *mint.address().as_array(),
        relay: expected_relay,
        price_zat,
        available: stock,
        stock,
        diversifier: arr(d, 44),
        svk_hash: arr(d, 55),
        vkey_hash: arr(d, 119),
        reservation_blocks,
    }
    .write(&mut machine_account.try_borrow_mut()?);
    Ok(())
}
fn reserve(id: &Address, accounts: &[AccountView], d: &[u8]) -> ProgramResult {
    let [payer, ma, ra, relay_account, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if d.len() != 32 {
        return Err(ProgramError::InvalidInstructionData);
    }
    signer(payer)?;
    writable(payer)?;
    writable(ma)?;
    program(system, &SYSTEM)?;
    let mut m = machine(id, ma)?;
    let rd = relay_account.try_borrow()?;
    let view = relay(relay_account, &m.relay, &rd)?;
    if m.available == 0 {
        return Err(fail(E::SoldOut));
    }
    let expiry_height = view
        .tip_height()
        .checked_add(m.reservation_blocks)
        .ok_or(fail(E::ArithmeticOverflow))?;
    drop(rd);
    let nonce = arr(d, 0);
    let bump = new_pda(
        payer,
        ra,
        id,
        RESERVATION_LEN,
        b"reservation",
        ma.address().as_array(),
        &nonce,
    )?;
    Reservation {
        bump,
        machine: *ma.address().as_array(),
        nonce,
        payout: *payer.address().as_array(),
        price_zat: m.price_zat,
        expiry_height,
        created_at: Clock::get()?.unix_timestamp,
    }
    .write(&mut ra.try_borrow_mut()?);
    m.available -= 1;
    m.write(&mut ma.try_borrow_mut()?);
    Ok(())
}
fn claim(id: &Address, accounts: &[AccountView], d: &[u8]) -> ProgramResult {
    let [submitter, ma, ra, relay_account, payout, mint, authority, destination, tag, token, associated, system] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if d.len() < 2 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let n = u16::from_le_bytes(arr(d, 0)) as usize;
    if d.len() != 2 + n + PUBLIC_VALUES_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    signer(submitter)?;
    writable(submitter)?;
    let m = machine(id, ma)?;
    let public = &d[2 + n..];
    let pv = PublicValues::decode(public).map_err(|_| fail(E::BadPublicValues))?;
    // Validate structural account bindings before verification; the remaining policy is pure.
    let r = reservation(id, ra, ma.address().as_array())?;
    expect_key(payout, &r.payout)?;
    expect_key(mint, &m.mint)?;
    let (auth, bump) = pda(&[b"mint", ma.address().as_ref()], id);
    expect_key(authority, auth.as_array())?;
    if bump != m.mint_bump {
        return Err(fail(E::WrongAccount));
    }
    program(token, &TOKEN)?;
    program(associated, &ATA)?;
    program(system, &SYSTEM)?;
    let expected_ata = pda(&[&r.payout, TOKEN.as_ref(), &m.mint], &ATA).0;
    expect_key(destination, expected_ata.as_array())?;
    let expected_tag = pda(&[b"tag", ma.address().as_ref(), &pv.payment_tag], id).0;
    expect_key(tag, expected_tag.as_array())?;
    sp1_solana_v6::verify_proof(&d[2..2 + n], public, &m.vkey_hash)
        .map_err(|_| fail(E::InvalidProof))?;
    let rd = relay_account.try_borrow()?;
    let plan = validate_claim(
        &pv,
        ClaimAccounts {
            machine_key: ma.address().as_array(),
            machine: &m,
            reservation: Some(&r),
            relay_owner: &owner(relay_account),
            relay_key: relay_account.address().as_array(),
            relay_data: &rd,
            tag_exists: tag.owned_by(id) && !tag.is_data_empty(),
        },
    )
    .map_err(fail)?;
    drop(rd);
    new_pda(
        submitter,
        tag,
        id,
        TAG_LEN,
        b"tag",
        ma.address().as_array(),
        &plan.payment_tag,
    )?;
    tag.try_borrow_mut()?.copy_from_slice(&[3, VERSION]);
    // Idempotent ATA creation validates existing accounts as well as creating a missing one.
    invoke_signed(
        &InstructionView {
            program_id: &ATA,
            data: &[1],
            accounts: &[
                Meta::writable_signer(submitter.address()),
                Meta::writable(destination.address()),
                Meta::readonly(payout.address()),
                Meta::readonly(mint.address()),
                Meta::readonly(&SYSTEM),
                Meta::readonly(&TOKEN),
            ],
        },
        &[submitter, destination, payout, mint, system, token],
        &[],
    )?;
    let mut mint_to = [0; 9];
    mint_to[0] = 7;
    mint_to[1..].copy_from_slice(&plan.tickets.to_le_bytes());
    invoke_signed(
        &InstructionView {
            program_id: &TOKEN,
            data: &mint_to,
            accounts: &[
                Meta::writable(mint.address()),
                Meta::writable(destination.address()),
                Meta::readonly_signer(authority.address()),
            ],
        },
        &[mint, destination, authority],
        &[Signer::from(&[
            Seed::from(b"mint"),
            Seed::from(ma.address().as_ref()),
            Seed::from(&[m.mint_bump]),
        ])],
    )?;
    close(ra, payout)
}
fn release(id: &Address, accounts: &[AccountView], d: &[u8]) -> ProgramResult {
    let [submitter, ma, ra, relay_account, payout] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !d.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    signer(submitter)?;
    writable(ma)?;
    let mut m = machine(id, ma)?;
    let r = reservation(id, ra, ma.address().as_array())?;
    expect_key(payout, &r.payout)?;
    let rd = relay_account.try_borrow()?;
    let view = RelayStatus::parse_allow_halted(
        &owner(relay_account),
        relay_account.address().as_array(),
        &m.relay,
        &rd,
    )
    .map_err(|e| fail(view_error(e)))?;
    can_release(
        view.tip_height(),
        view.finality_depth(),
        view.is_halted(),
        r.expiry_height,
        r.created_at,
        Clock::get()?.unix_timestamp,
    )
    .map_err(fail)?;
    drop(rd);
    m.available = m
        .available
        .checked_add(1)
        .filter(|v| *v <= m.stock)
        .ok_or(fail(E::ArithmeticOverflow))?;
    close(ra, payout)?;
    m.write(&mut ma.try_borrow_mut()?);
    Ok(())
}
