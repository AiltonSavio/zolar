#![allow(clippy::result_large_err)]
//! LiteSVM harness for the Escrow program. It drives the Escrow and the Header Relay only through
//! their generated clients; token accounts are written directly in the SPL Token layout.

use std::{path::PathBuf, process::Command, sync::Once};

use escrow_client::{accounts::Escrow, instructions::*, ESCROW_ID};
use header_relay_interface::RelayView;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use relay_tests::{
    synthetic::{synthetic_checkpoint, Branch, Synth, CHECKPOINT_HEIGHT},
    Harness,
};
use solana_account::Account;
use solana_address::Address;
use solana_instruction::{error::InstructionError, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;
use zolar_claim::{block_leaf, payment_commitment, tx_merkle_path, tx_merkle_root, FinalBlockTree};

pub use relay_tests::{set_compute_limit, CU_CAP};

pub const TOKEN_PROGRAM: Address =
    Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);

fn escrow_so() -> PathBuf {
    static BUILD: Once = Once::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../programs/escrow");
    BUILD.call_once(|| {
        let status = Command::new("cargo")
            .args(["build-sbf", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .status()
            .expect("cargo build-sbf must be installed");
        assert!(status.success(), "cargo build-sbf failed");
    });
    root.join("target/deploy/escrow.so")
}

/// An initialised SPL Token account (165 bytes).
pub fn token_account(mint: &Address, owner: &Address, amount: u64) -> Account {
    let mut d = vec![0u8; 165];
    d[0..32].copy_from_slice(mint.as_ref());
    d[32..64].copy_from_slice(owner.as_ref());
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1;
    Account {
        lamports: 2_039_280,
        data: d,
        owner: TOKEN_PROGRAM,
        executable: false,
        rent_epoch: 0,
    }
}

/// An initialised SPL Token mint (82 bytes) without a freeze authority.
pub fn mint_account(decimals: u8) -> Account {
    let mut d = vec![0u8; 82];
    d[44] = decimals;
    d[45] = 1;
    Account {
        lamports: 1_461_600,
        data: d,
        owner: TOKEN_PROGRAM,
        executable: false,
        rent_epoch: 0,
    }
}

pub fn token_balance(h: &Harness, account: &Address) -> Option<u64> {
    h.svm.get_account(account).and_then(|a| {
        (a.owner == TOKEN_PROGRAM && a.data.len() == 165)
            .then(|| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
    })
}

/// Everything one trade needs, fixed when the Escrow is created.
#[derive(Clone)]
pub struct Deal {
    pub txids: Vec<[u8; 32]>,
    /// Position of the payment in its block.
    pub index: usize,
    pub salt: [u8; 32],
    pub commitment: [u8; 32],
    pub escrow: Address,
    pub vault: Address,
    pub amount: u64,
    pub expiry: u32,
}

impl Deal {
    pub fn txid(&self) -> [u8; 32] {
        self.txids[self.index]
    }
}

pub struct World {
    pub h: Harness,
    pub relay: Address,
    pub branch: Branch,
    pub depth: u16,
    pub payee: Keypair,
    /// The Payer's wallet; the Escrow pays token accounts it owns.
    pub payer: Keypair,
    pub mint: Address,
    pub payee_token: Address,
    pub payer_token: Address,
    next_tag: u32,
}

pub const PAYEE_FUNDS: u64 = 5_000_000;

impl World {
    /// A Header Relay at Finality Depth `depth` holding two blocks above its Checkpoint, an Escrow
    /// program, a mint, and funded token accounts for a Payee and a Payer.
    pub fn new(depth: u16) -> Self {
        let mut h = Harness::new();
        h.svm
            .add_program_from_file(ESCROW_ID, escrow_so())
            .expect("escrow.so");
        let checkpoint = synthetic_checkpoint(depth, 1);
        let relay = h.init_relay(&checkpoint);
        let branch = Branch::new(&checkpoint);
        let payee = Keypair::new();
        let payer = Keypair::new();
        h.svm.airdrop(&payee.pubkey(), 10_000_000_000).unwrap();
        let mint = Address::new_unique();
        h.svm.set_account(mint, mint_account(6)).unwrap();
        let payee_token = Address::new_unique();
        h.svm
            .set_account(
                payee_token,
                token_account(&mint, &payee.pubkey(), PAYEE_FUNDS),
            )
            .unwrap();
        let payer_token = Address::new_unique();
        h.svm
            .set_account(payer_token, token_account(&mint, &payer.pubkey(), 0))
            .unwrap();
        let mut w = Self {
            h,
            relay,
            branch,
            depth,
            payee,
            payer,
            mint,
            payee_token,
            payer_token,
            next_tag: 0,
        };
        w.mine(2);
        w
    }

    /// Relays `n` ordinary blocks.
    pub fn mine(&mut self, n: u32) {
        for _ in 0..n {
            self.next_tag += 1;
            let block = self.branch.next(self.next_tag);
            self.relay_block(&block);
        }
    }

    /// Relays a block that commits to the transaction Merkle root of `txids`.
    pub fn mine_block_with(&mut self, txids: &[[u8; 32]]) -> Synth {
        let block = self.branch.next_with_root(tx_merkle_root(txids));
        self.relay_block(&block);
        block
    }

    pub fn relay_block(&mut self, block: &Synth) {
        self.h
            .relay_synthetic(&self.relay, &block.raw)
            .expect("the relay accepts the block");
    }

    pub fn tip(&self) -> u32 {
        let data = self.h.relay_data(&self.relay);
        RelayView::parse(&data).unwrap().tip_height()
    }

    /// Blocks needed above `height` for it to be in the Final Block Tree.
    pub fn finalize(&mut self, height: u32) {
        while self.tip() < height + self.depth as u32 {
            self.mine(1);
        }
    }

    pub fn deal(&self, txids: Vec<[u8; 32]>, index: usize, salt: u8, expiry: u32) -> Deal {
        let salt = {
            let mut s = [0u8; 32];
            s[31] = salt;
            s[0] = 1;
            s
        };
        let commitment = payment_commitment(&txids[index], &salt).unwrap();
        let (escrow, _) = Escrow::find_pda(&self.payee.pubkey(), commitment);
        Deal {
            txids,
            index,
            salt,
            commitment,
            escrow,
            vault: Address::new_unique(),
            amount: 1_000_000,
            expiry,
        }
    }

    /// Creates the vault account (owner = the Escrow PDA) and returns the CreateEscrow instruction.
    pub fn create_ix(&mut self, deal: &Deal) -> Instruction {
        self.h
            .svm
            .set_account(deal.vault, token_account(&self.mint, &deal.escrow, 0))
            .unwrap();
        self.create_ix_with(deal, deal.amount, 0, 0)
    }

    pub fn create_ix_with(
        &self,
        deal: &Deal,
        amount: u64,
        bond_required: u64,
        claim_mode: u8,
    ) -> Instruction {
        CreateEscrowBuilder::new()
            .payee(self.payee.pubkey())
            .escrow(deal.escrow)
            .payee_token(self.payee_token)
            .vault(deal.vault)
            .relay(self.relay)
            .commitment(deal.commitment)
            .payout(self.payer.pubkey())
            .amount(amount)
            .bond_required(bond_required)
            .expiry_height(deal.expiry)
            .claim_mode(claim_mode)
            .instruction()
    }

    pub fn create(&mut self, deal: &Deal) -> u64 {
        let ix = self.create_ix(deal);
        let payee = self.payee.insecure_clone();
        self.h.ok(&[set_compute_limit(CU_CAP), ix], &[&payee]) - 150
    }

    pub fn create_err(&mut self, ix: Instruction) -> u32 {
        let payee = self.payee.insecure_clone();
        self.h.custom_err(&[ix], &[&payee])
    }

    /// The Final Block Tree rebuilt from the blocks the relay holds as final.
    pub fn tree(&self) -> (FinalBlockTree, usize) {
        let data = self.h.relay_data(&self.relay);
        let size = RelayView::parse(&data).unwrap().leaf_count() as usize;
        let leaves = self.branch.blocks[..size]
            .iter()
            .map(|b| block_leaf(b.height, &b.merkle_root).unwrap())
            .collect();
        (FinalBlockTree::new(leaves), size)
    }

    /// The Reveal Claim for the payment in the block at `block_height`, against the tree of `size`
    /// leaves (`None`: the newest).
    pub fn claim_ix(&self, deal: &Deal, block_height: u32, size: Option<usize>) -> Instruction {
        let (tree, full) = self.tree();
        let size = size.unwrap_or(full);
        let index = (block_height - CHECKPOINT_HEIGHT - 1) as usize;
        let path = tree.compress(tree.path(index, size));
        let root = tree.root(size);
        self.claim_ix_raw(deal, block_height, root, path, deal.index)
    }

    pub fn claim_ix_raw(
        &self,
        deal: &Deal,
        block_height: u32,
        root: [u8; 32],
        tree_path: Vec<[u8; 32]>,
        tx_index: usize,
    ) -> Instruction {
        RevealClaimBuilder::new()
            .escrow(deal.escrow)
            .relay(self.relay)
            .vault(deal.vault)
            .payout_token(self.payer_token)
            .payee(self.payee.pubkey())
            .txid(deal.txid())
            .salt(deal.salt)
            .block_height(block_height)
            .tx_index(tx_index as u32)
            .root(root)
            .tree_path(tree_path.into())
            .tx_path(tx_merkle_path(&deal.txids, tx_index).into())
            .instruction()
    }

    pub fn claim(&mut self, ix: Instruction) -> u64 {
        self.h.ok(&[set_compute_limit(CU_CAP), ix], &[]) - 150
    }

    pub fn claim_err(&mut self, ix: Instruction) -> u32 {
        self.h.custom_err(&[ix], &[])
    }

    pub fn try_send(
        &mut self,
        ix: Instruction,
    ) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        self.h.send(&[set_compute_limit(CU_CAP), ix], &[])
    }
}

/// The custom error code of a failed transaction, if it is one.
pub fn custom_code(e: &FailedTransactionMetadata) -> Option<u32> {
    match &e.err {
        TransactionError::InstructionError(_, InstructionError::Custom(code)) => Some(*code),
        _ => None,
    }
}

/// A block of `n` distinct transaction IDs.
pub fn txids(n: u8) -> Vec<[u8; 32]> {
    (0..n)
        .map(|i| {
            let mut t = [0u8; 32];
            t[0] = i;
            t[1] = 0xA5;
            t[31] = i.wrapping_mul(7).wrapping_add(3);
            t
        })
        .collect()
}
