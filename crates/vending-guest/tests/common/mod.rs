use orchard::{
    keys::IncomingViewingKey,
    note::{ExtractedNoteCommitment, NoteVersion, RandomSeed, TransmittedNoteCiphertext},
    note_encryption::{OrchardDomain, OrchardNoteEncryption},
    value::NoteValue,
    Note,
};
use vending_guest::Witness;
use vending_types::Memo;
use zcash_note_encryption::Domain;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;

pub fn real_tx() -> Vec<u8> {
    hex::decode(include_str!("../../fixtures/ticket02-tx.hex").trim()).unwrap()
}
pub fn ivk() -> Vec<u8> {
    hex::decode(include_str!("../../fixtures/shop.ivk.hex").trim()).unwrap()
}
fn replace(tx: &mut [u8], old: &[u8], new: &[u8]) {
    assert_eq!(old.len(), new.len());
    let at = tx.windows(old.len()).position(|w| w == old).unwrap();
    tx[at..at + new.len()].copy_from_slice(new);
}

/// Changes a recorded transaction's output without creating valid authorization.
/// These fixtures are never consensus-valid payments and must never be broadcast.
pub fn fixture(kind: &str) -> Witness {
    let original = real_tx();
    let parsed = Transaction::read(original.as_slice(), BranchId::Nu7).unwrap();
    let bundle = parsed.ironwood_bundle().unwrap();
    let ivk_bytes: [u8; 64] = ivk().try_into().unwrap();
    let key = IncomingViewingKey::from_bytes(&ivk_bytes).unwrap();
    let (idx, _, old_note, _, _) = bundle.decrypt_outputs_with_keys(&[key]).remove(0);
    let action = &bundle.actions()[idx];
    let mut memo = Memo {
        machine: [2; 32],
        nonce: [3; 32],
    }
    .encode();
    if kind == "memo" {
        memo[1] = 2;
    }
    let rho = old_note.rho();
    let seed = if kind == "rseed" {
        RandomSeed::from_bytes([0x42; 32], &rho).unwrap()
    } else {
        *old_note.rseed()
    };
    let value = if kind == "value" {
        9_999_999
    } else {
        old_note.value().inner()
    };
    let note = Note::from_parts(
        old_note.recipient(),
        NoteValue::from_raw(value),
        rho,
        seed,
        NoteVersion::V3,
    )
    .unwrap();
    let cmx = ExtractedNoteCommitment::from(note.commitment()).to_bytes();
    let enc = OrchardNoteEncryption::new(None, note, memo);
    let encrypted = TransmittedNoteCiphertext {
        epk_bytes: OrchardDomain::epk_bytes(enc.epk()).0,
        enc_ciphertext: enc.encrypt_note_plaintext(),
        out_ciphertext: action.encrypted_note().out_ciphertext,
    };
    let mut tx = original.clone();
    let old = action.encrypted_note();
    replace(&mut tx, &old.epk_bytes, &encrypted.epk_bytes);
    replace(
        &mut tx,
        old.enc_ciphertext.as_ref(),
        encrypted.enc_ciphertext.as_ref(),
    );
    // For value/rseed, keep the old cmx while encrypting a different plaintext.
    if kind != "value" && kind != "rseed" {
        replace(&mut tx, &action.cmx().to_bytes(), &cmx);
    }
    if kind == "rho" {
        replace(&mut tx, &action.nullifier().to_bytes(), &[1; 32]);
    }
    let mut witness = Witness {
        tx,
        shop_viewing_key: ivk(),
        action_index: idx as u32,
        machine: [2; 32],
        nonce: [3; 32],
        diversifier: *old_note.recipient().diversifier().as_array(),
        tx_index: 0,
        tx_count: 1,
        tx_path: vec![],
        block_merkle_root: [0; 32],
        height: 4_515_934,
        tree_index: 0,
        tree_path: [[0; 32]; 20],
        root: [0; 32],
    };
    rebind(&mut witness);
    witness
}

pub fn rebind(w: &mut Witness) {
    let tx = Transaction::read(w.tx.as_slice(), BranchId::Nu7).unwrap();
    let txids = [*tx.txid().as_ref(), [9; 32], [8; 32]];
    w.tx_index = 0;
    w.tx_count = 3;
    w.tx_path = zolar_claim::tx_merkle_path(&txids, 0);
    w.block_merkle_root = zolar_claim::tx_merkle_root(&txids);
    let leaf = zolar_claim::block_leaf(w.height, &w.block_merkle_root).unwrap();
    let tree = zolar_claim::FinalBlockTree::new(vec![leaf]);
    w.tree_path = tree.path(0, 1).try_into().unwrap();
    w.root = tree.root(1);
}
