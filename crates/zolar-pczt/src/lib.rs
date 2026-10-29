//! Transaction ID and viewing-key output decryption for unsigned Ironwood PCZTs.
//!
//! The Escrow relies on the Payer knowing a Zcash transaction ID before signing. The ID of a
//! v5/v6 transaction commits to the effects only (ZIP 244), so it is fixed once the PCZT is created
//! and does not change when the PCZT is proven, signed or extracted.

use orchard::keys::{IncomingViewingKey, Scope};
mod zkool_wire;
use pczt::{Pczt, roles::redactor::Redactor};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::transaction::{
    TransactionData, TxVersion,
    txid::{TxIdDigester, to_txid},
};
use zcash_protocol::{
    consensus::{MAIN_NETWORK, TEST_NETWORK},
    memo::Memo,
};
pub use zkool_wire::inspection_pczt as zkool_inspection_pczt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Testnet,
    Mainnet,
}

/// An output the viewing key can decrypt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub zatoshis: u64,
    /// The text memo, or `None` for an empty or non-text memo.
    pub memo: Option<String>,
    pub pool: &'static str,
}

/// What anyone holding the PCZT can read without a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// Transaction ID in the conventional display (byte-reversed hex) form.
    pub txid: String,
    /// Transaction format version: 5 or 6.
    pub tx_version: u8,
    /// The last block the transaction can be mined in.
    pub expiry_height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Transaction ID in the conventional display (byte-reversed hex) form.
    pub txid: String,
    /// Transaction format version: 5 or 6.
    pub tx_version: u8,
    /// The last block the transaction can be mined in.
    pub expiry_height: u32,
    /// Outputs addressed to the viewing key (external scope), in action order.
    pub received: Vec<Output>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a valid PCZT: {0}")]
    Pczt(String),
    #[error("not enough information to build the transaction effects: {0}")]
    Effects(String),
    #[error("not a valid unified full viewing key for this network: {0}")]
    ViewingKey(String),
    #[error("unsupported transaction version")]
    Version,
}

/// The transaction ID, version and expiry height of `pczt`.
pub fn summary(pczt: &[u8]) -> Result<Summary, Error> {
    let (_, summary) = parse(pczt)?;
    Ok(summary)
}

/// External-wallet demo funding must not silently use transparent inputs.
pub fn shielded_v6(bytes: &[u8]) -> Result<bool, Error> {
    let pczt = Pczt::parse(bytes).map_err(|e| Error::Pczt(format!("{e:?}")))?;
    Ok(pczt.transparent().inputs().is_empty() && summary(bytes)?.tx_version == 6)
}

fn parse(pczt: &[u8]) -> Result<(TransactionData<pczt::EffectsOnly>, Summary), Error> {
    let pczt = Pczt::parse(pczt).map_err(|e| Error::Pczt(format!("{e:?}")))?;
    let tx = pczt
        .into_effects()
        .map_err(|e| Error::Effects(format!("{e:?}")))?;
    let parts = tx.digest(TxIdDigester);
    let txid = to_txid(tx.version(), tx.consensus_branch_id(), &parts).to_string();
    let tx_version = match tx.version() {
        TxVersion::V5 => 5,
        TxVersion::V6 => 6,
        _ => return Err(Error::Version),
    };
    let expiry_height = u32::from(tx.expiry_height());
    Ok((
        tx,
        Summary {
            txid,
            tx_version,
            expiry_height,
        },
    ))
}

/// Computes the transaction ID of `pczt` and decrypts the Orchard and Ironwood outputs addressed
/// to `ufvk`. Needs no spending key.
pub fn inspect(pczt: &[u8], ufvk: &str, network: Network) -> Result<Report, Error> {
    let ufvk = match network {
        Network::Testnet => UnifiedFullViewingKey::decode(&TEST_NETWORK, ufvk),
        Network::Mainnet => UnifiedFullViewingKey::decode(&MAIN_NETWORK, ufvk),
    }
    .map_err(|e| Error::ViewingKey(e.to_string()))?;
    let ivk: Option<IncomingViewingKey> = ufvk.orchard().map(|f| f.to_ivk(Scope::External));

    let (tx, summary) = parse(pczt)?;

    let mut received = Vec::new();
    if let Some(ivk) = ivk {
        let pools = [
            ("orchard", tx.orchard_bundle()),
            ("ironwood", tx.ironwood_bundle()),
        ];
        for (pool, bundle) in pools {
            let Some(bundle) = bundle else { continue };
            for (_, _, note, _, memo) in
                bundle.decrypt_outputs_with_keys(std::slice::from_ref(&ivk))
            {
                let memo = match Memo::from_bytes(&memo) {
                    Ok(Memo::Text(text)) => Some(String::from(text)),
                    _ => None,
                };
                received.push(Output {
                    zatoshis: note.value().inner(),
                    memo,
                    pool,
                });
            }
        }
    }

    Ok(Report {
        txid: summary.txid,
        tx_version: summary.tx_version,
        expiry_height: summary.expiry_height,
        received,
    })
}

/// The copy of an unsigned PCZT to hand the Payee: the same transaction effects, so the same
/// transaction ID and the same outputs for its viewing key to decrypt, without anything private to the
/// Payer. An unredacted PCZT carries, for every spend, the note's value, the address that received it,
/// its randomness, a Merkle witness (its position in the note tree), the ZIP 32 derivation path and the
/// full viewing key of the spending account, and for every output (the change too) its recipient,
/// value and randomness. The Payer keeps the full PCZT to prove and sign.
pub fn redact_for_payee(pczt: &[u8]) -> Result<Vec<u8>, Error> {
    let mut pczt = Pczt::parse(pczt).map_err(|e| Error::Pczt(format!("{e:?}")))?;
    // Make derived fields explicit first (note and value commitments, encrypted note plaintexts):
    // once the private fields are gone they could not be derived again.
    pczt.resolve_fields()
        .map_err(|e| Error::Pczt(format!("resolving fields: {e:?}")))?;
    let redact = |mut bundle: pczt::roles::redactor::orchard::OrchardRedactor<'_>| {
        bundle.clear_bsk();
        bundle.redact_actions(|mut action| {
            action.clear_spend_recipient();
            action.clear_spend_value();
            action.clear_spend_rho();
            action.clear_spend_rseed();
            action.clear_spend_fvk();
            action.clear_spend_witness();
            action.clear_spend_alpha();
            action.clear_spend_zip32_derivation();
            action.clear_spend_dummy_sk();
            action.clear_spend_proprietary();
            action.clear_output_recipient();
            action.clear_output_value();
            action.clear_output_rseed();
            action.clear_output_ock();
            action.clear_output_zip32_derivation();
            action.clear_output_user_address();
            action.clear_output_proprietary();
            action.clear_rcv();
        });
    };
    let redacted = Redactor::new(pczt)
        .redact_global_with(|mut global| global.clear_proprietary())
        .redact_transparent_with(|mut bundle| {
            bundle.redact_inputs(|mut input| {
                input.clear_redeem_script();
                input.clear_partial_signatures();
                input.clear_bip32_derivation();
                input.clear_ripemd160_preimages();
                input.clear_sha256_preimages();
                input.clear_hash160_preimages();
                input.clear_hash256_preimages();
                input.clear_proprietary();
            });
            bundle.redact_outputs(|mut output| {
                output.clear_redeem_script();
                output.clear_bip32_derivation();
                output.clear_user_address();
                output.clear_proprietary();
            });
        })
        .redact_sapling_with(|mut bundle| {
            bundle.clear_bsk();
            bundle.redact_spends(|mut spend| {
                spend.clear_recipient();
                spend.clear_value();
                spend.clear_rcm();
                spend.clear_rseed();
                spend.clear_rcv();
                spend.clear_proof_generation_key();
                spend.clear_witness();
                spend.clear_alpha();
                spend.clear_zip32_derivation();
                spend.clear_dummy_ask();
                spend.clear_proprietary();
            });
            bundle.redact_outputs(|mut output| {
                output.clear_recipient();
                output.clear_value();
                output.clear_rseed();
                output.clear_rcv();
                output.clear_ock();
                output.clear_zip32_derivation();
                output.clear_user_address();
                output.clear_proprietary();
            });
        })
        .redact_orchard_with(redact)
        .redact_ironwood_with(redact)
        .finish();
    redacted
        .serialize()
        .map_err(|e| Error::Pczt(format!("serializing: {e:?}")))
}
