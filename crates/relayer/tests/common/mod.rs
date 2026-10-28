#![allow(dead_code)]
use std::{cell::RefCell, rc::Rc};

use litesvm::LiteSVM;
use solana_address::Address;
use solana_instruction::{error::InstructionError, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;
use zolar_relayer::{
    chain::{Cluster, TxError},
    follower::{Batch, Hash, HeaderSource},
    p2p::{sha256d, RawHeader},
};

/// LiteSVM as a cluster. `fail_sends` lists send counts (1-based) that fail like a dropped connection.
pub struct Lite {
    pub svm: Rc<RefCell<LiteSVM>>,
    pub sends: usize,
    pub fail_sends: Vec<usize>,
}

impl Cluster for Lite {
    fn send(&mut self, ixs: &[Instruction], payer: &Keypair) -> Result<String, TxError> {
        self.sends += 1;
        if self.fail_sends.contains(&self.sends) {
            return Err(TxError::Other("connection reset".into()));
        }
        let mut svm = self.svm.borrow_mut();
        svm.expire_blockhash();
        let tx = Transaction::new(
            &[payer],
            Message::new(ixs, Some(&payer.pubkey())),
            svm.latest_blockhash(),
        );
        let signature = tx.signatures[0].to_string();
        match svm.send_transaction(tx) {
            Ok(_) => Ok(signature),
            Err(e) => match e.err {
                TransactionError::InstructionError(index, InstructionError::Custom(code)) => {
                    Err(TxError::Program {
                        index: index as usize,
                        code,
                    })
                }
                other => Err(TxError::Other(format!("{other:?}"))),
            },
        }
    }

    fn account_with_owner(
        &mut self,
        address: &Address,
    ) -> Result<Option<(Address, Vec<u8>)>, String> {
        Ok(self
            .svm
            .borrow()
            .get_account(address)
            .map(|a| (a.owner, a.data)))
    }
}

pub struct Rows {
    pub rows: Vec<RawHeader>,
    /// Replaces the header at this index with another one (a forged block at the same position).
    pub forged: Option<(usize, RawHeader)>,
}

impl HeaderSource for Rows {
    fn headers_after(&mut self, locator: &[Hash]) -> Result<Batch, String> {
        let mut rows = self.rows.clone();
        if let Some((i, h)) = self.forged {
            rows[i] = h;
        }
        let hashes: Vec<Hash> = self.rows.iter().map(|h| sha256d(h)).collect();
        let start = locator
            .iter()
            .find_map(|l| hashes.iter().position(|h| h == l))
            .map_or(0, |i| i + 1);
        Ok(Batch {
            peer: "fixture".into(),
            headers: rows[start..].to_vec(),
        })
    }
    fn penalize(&mut self, _: &str) {}
}
