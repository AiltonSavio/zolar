//! The complete vending payment statement, shared by native checks and the SP1 guest.
//! A successful check is meaningful only when the committed root is accepted by the Header Relay.
use orchard::keys::IncomingViewingKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vending_types::{Memo, PublicValues};
use zcash_primitives::transaction::{Transaction, TxVersion};
use zcash_protocol::consensus::BranchId;
use zolar_claim::{block_leaf, fold_tx_path, poseidon, DEPTH};

/// Hashes and txids use protocol byte order. Poseidon elements use Relay big-endian byte order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Witness {
    pub tx: Vec<u8>,
    /// Exactly 64 bytes, `dk || ivk`. A Vec keeps serde/SP1's encoding portable.
    pub shop_viewing_key: Vec<u8>,
    pub action_index: u32,
    pub machine: [u8; 32],
    pub nonce: [u8; 32],
    pub payout: [u8; 32],
    pub diversifier: [u8; 11],
    pub tx_index: u32,
    pub tx_count: u32,
    pub tx_path: Vec<[u8; 32]>,
    pub block_merkle_root: [u8; 32],
    pub height: u32,
    pub tree_index: u32,
    pub tree_path: [[u8; 32]; DEPTH],
    pub root: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transaction,
    ViewingKey,
    Action,
    Decryption,
    Diversifier,
    Memo,
    Machine,
    Nonce,
    Payout,
    TxPath,
    TreePath,
}

#[derive(Clone, Debug)]
pub struct Payment {
    pub txid: [u8; 32],
    pub branch_id: u32,
    pub value_zat: u64,
    pub payment_tag: [u8; 32],
    pub diversifier: [u8; 11],
    pub memo: [u8; 512],
}

/// Full trial decryption checks the AEAD tag, derived epk, and recomputed cmx (including
/// Ironwood's binding of value, rho, rseed and diversified recipient). No plaintext is a witness.
pub fn decrypt_payment(raw: &[u8], key: &[u8], action_index: u32) -> Result<Payment, Error> {
    if raw.len() > 2_000_000 {
        return Err(Error::Transaction);
    }
    let mut rest = raw;
    let tx = Transaction::read(&mut rest, BranchId::Nu7).map_err(|_| Error::Transaction)?;
    if !rest.is_empty() || tx.version() != TxVersion::V6 {
        return Err(Error::Transaction);
    }
    let key_bytes: &[u8; 64] = key.try_into().map_err(|_| Error::ViewingKey)?;
    let ivk: IncomingViewingKey =
        Option::from(IncomingViewingKey::from_bytes(key_bytes)).ok_or(Error::ViewingKey)?;
    let bundle = tx.ironwood_bundle().ok_or(Error::Action)?;
    if action_index as usize >= bundle.actions().len() {
        return Err(Error::Action);
    }
    for (idx, _, note, recipient, memo) in bundle.decrypt_outputs_with_keys(&[ivk]) {
        if idx != action_index as usize {
            continue;
        }
        return Ok(Payment {
            txid: *tx.txid().as_ref(),
            branch_id: tx.consensus_branch_id().into(),
            value_zat: note.value().inner(),
            payment_tag: bundle.actions()[idx].nullifier().to_bytes(),
            diversifier: *recipient.diversifier().as_array(),
            memo,
        });
    }
    Err(Error::Decryption)
}

/// Validate the SHA-256d path's shape and odd-node duplication as well as its root.
pub fn check_tx_path(txid: &[u8; 32], w: &Witness) -> Result<(), Error> {
    if w.tx_count == 0 || w.tx_index >= w.tx_count || w.tx_path.len() > 32 {
        return Err(Error::TxPath);
    }
    let mut count = w.tx_count;
    let mut index = w.tx_index;
    let mut node = *txid;
    for sibling in &w.tx_path {
        if count <= 1 {
            return Err(Error::TxPath);
        }
        let duplicated = index & 1 == 0 && index + 1 == count;
        if (node == *sibling) != duplicated {
            return Err(Error::TxPath);
        }
        node = if index & 1 == 0 {
            zolar_claim::sha256d_pair(&node, sibling)
        } else {
            zolar_claim::sha256d_pair(sibling, &node)
        };
        count = count.div_ceil(2);
        index >>= 1;
    }
    if count != 1 || fold_tx_path(txid, w.tx_index as usize, &w.tx_path) != w.block_merkle_root {
        return Err(Error::TxPath);
    }
    Ok(())
}

pub fn verify(w: &Witness) -> Result<PublicValues, Error> {
    let payment = decrypt_payment(&w.tx, &w.shop_viewing_key, w.action_index)?;
    check_tx_path(&payment.txid, w)?;
    if w.tree_index >= 1 << DEPTH {
        return Err(Error::TreePath);
    }
    let mut node = block_leaf(w.height, &w.block_merkle_root).map_err(|_| Error::TreePath)?;
    for (level, sibling) in w.tree_path.iter().enumerate() {
        node = if (w.tree_index >> level) & 1 == 0 {
            poseidon(&[&node, sibling])
        } else {
            poseidon(&[sibling, &node])
        }
        .map_err(|_| Error::TreePath)?;
    }
    if node != w.root {
        return Err(Error::TreePath);
    }
    // The decrypted address was constructed from DiversifyHash(plaintext d). Equality of d
    // binds the committed diversifier to that exact group element and transmission key.
    if payment.diversifier != w.diversifier {
        return Err(Error::Diversifier);
    }
    let memo = Memo::decode(&payment.memo).map_err(|_| Error::Memo)?;
    if memo.machine != w.machine {
        return Err(Error::Machine);
    }
    if memo.nonce != w.nonce {
        return Err(Error::Nonce);
    }
    if memo.payout != w.payout {
        return Err(Error::Payout);
    }
    Ok(PublicValues {
        branch_id: payment.branch_id,
        machine: memo.machine,
        nonce: memo.nonce,
        payout: memo.payout,
        value_zat: payment.value_zat,
        payment_tag: payment.payment_tag,
        root: w.root,
        height: w.height,
        diversifier: payment.diversifier,
        svk_hash: Sha256::digest(&w.shop_viewing_key).into(),
    })
}
