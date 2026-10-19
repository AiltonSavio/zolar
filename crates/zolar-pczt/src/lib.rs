//! Transaction ID and viewing-key output decryption for unsigned Ironwood PCZTs.
//!
//! The Escrow relies on the Payer knowing a Zcash transaction ID before signing. The ID of a
//! v5/v6 transaction commits to the effects only (ZIP 244), so it is fixed once the PCZT is created
//! and does not change when the PCZT is proven, signed or extracted.

use orchard::keys::{IncomingViewingKey, Scope};
use pczt::Pczt;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::transaction::{
    TxVersion,
    txid::{TxIdDigester, to_txid},
};
use zcash_protocol::{
    consensus::{MAIN_NETWORK, TEST_NETWORK},
    memo::Memo,
};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Transaction ID in the conventional display (byte-reversed hex) form.
    pub txid: String,
    /// Transaction format version: 5 or 6.
    pub tx_version: u8,
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

/// Computes the transaction ID of `pczt` and decrypts the Orchard and Ironwood outputs addressed
/// to `ufvk`. Needs no spending key.
pub fn inspect(pczt: &[u8], ufvk: &str, network: Network) -> Result<Report, Error> {
    let ufvk = match network {
        Network::Testnet => UnifiedFullViewingKey::decode(&TEST_NETWORK, ufvk),
        Network::Mainnet => UnifiedFullViewingKey::decode(&MAIN_NETWORK, ufvk),
    }
    .map_err(|e| Error::ViewingKey(e.to_string()))?;
    let ivk: Option<IncomingViewingKey> = ufvk.orchard().map(|f| f.to_ivk(Scope::External));

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
        txid,
        tx_version,
        received,
    })
}
