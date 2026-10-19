//! Cheap, valid-looking headers for tests of chain selection, eviction and Halt, which need many
//! competing branches that no fixture holds.
//!
//! The headers are real in everything the relay checks except the Equihash solution: the proof of
//! work is ground (the test network's easy targets need a few hundred hashes), the difficulty comes
//! from the same adjustment rules the program uses, and the verifier state of the upload buffer is set
//! to "verified" directly instead of being run (that part is covered by the live-header replays).
//! The solution bytes are zeros. Use only where the Equihash check is not what the test is about.

use header_relay_client::types::Checkpoint;
use solana_address::Address;
use solana_signer::Signer;
use zcash_header::{
    difficulty::{expected_bits, Rules, MEDIAN_SPAN},
    header::HEADER_LEN,
    pow::{check_pow, Network},
};

use crate::{header_bytes_hash, set_compute_limit, Harness};

/// `nBits` of ordinary blocks in the synthetic Checkpoint: target about 2^247, a few hundred hashes.
pub const NORMAL_BITS: u32 = 0x1f7f_ffff;
/// Seconds between ordinary blocks before NU7.
pub const SPACING: u32 = 75;
/// A gap that lets a testnet block use the minimum difficulty.
pub const MIN_DIFFICULTY_GAP: u32 = 451;
pub const CHECKPOINT_HEIGHT: u32 = 4_000_000;
const VERIFIER_OFFSET: usize = 67 + HEADER_LEN;

#[derive(Clone)]
pub struct Synth {
    pub raw: Vec<u8>,
    pub hash: [u8; 32],
    pub height: u32,
    pub merkle_root: [u8; 32],
    pub bits: u32,
}

/// A testnet Checkpoint of 113 blocks at `SPACING` and `NORMAL_BITS`; NU7 never activates.
pub fn synthetic_checkpoint(finality_depth: u16, nonce: u64) -> Checkpoint {
    let times: Vec<u32> = (0..113).map(|i| 1_700_000_000 + i * SPACING).collect();
    Checkpoint {
        version: 1,
        network: 1,
        finality_depth,
        nu7_height: u32::MAX,
        height: CHECKPOINT_HEIGHT,
        nonce,
        hash: header_bytes_hash(b"synthetic checkpoint block"),
        times: times.try_into().unwrap(),
        bits: [NORMAL_BITS; 113],
    }
}

/// One chain of synthetic blocks above a Checkpoint; `fork_at` copies it up to a block.
#[derive(Clone)]
pub struct Branch {
    rules: Rules,
    times: Vec<u32>,
    bits: Vec<u32>,
    pub blocks: Vec<Synth>,
    tip_hash: [u8; 32],
    tip_height: u32,
}

impl Branch {
    pub fn new(checkpoint: &Checkpoint) -> Self {
        Self {
            rules: Rules {
                network: Network::Testnet,
                nu7_height: checkpoint.nu7_height,
            },
            times: checkpoint.times.to_vec(),
            bits: checkpoint.bits.to_vec(),
            blocks: Vec::new(),
            tip_hash: checkpoint.hash,
            tip_height: checkpoint.height,
        }
    }

    pub fn tip_height(&self) -> u32 {
        self.tip_height
    }

    /// The branch that ends at the block of height `height` (the Checkpoint's own height keeps none).
    pub fn fork_at(&self, height: u32) -> Self {
        let keep = (height - CHECKPOINT_HEIGHT) as usize;
        let mut fork = Self::new_like(self);
        for block in &self.blocks[..keep] {
            fork.push_known(block.clone(), self.time_of(block.height), block.bits);
        }
        fork
    }

    fn new_like(other: &Self) -> Self {
        let n = other.times.len() - other.blocks.len();
        let first = other
            .blocks
            .first()
            .map(|b| b.raw[4..36].try_into().unwrap());
        Self {
            rules: other.rules,
            times: other.times[..n].to_vec(),
            bits: other.bits[..n].to_vec(),
            blocks: Vec::new(),
            tip_hash: first.unwrap_or(other.tip_hash),
            tip_height: CHECKPOINT_HEIGHT,
        }
    }

    fn time_of(&self, height: u32) -> u32 {
        let n = self.times.len() - self.blocks.len();
        self.times[n + (height - CHECKPOINT_HEIGHT) as usize - 1]
    }

    fn push_known(&mut self, block: Synth, time: u32, bits: u32) {
        self.tip_hash = block.hash;
        self.tip_height = block.height;
        self.times.push(time);
        self.bits.push(bits);
        self.blocks.push(block);
    }

    /// The next block of this branch, `gap` seconds after its parent, made unique by `tag`.
    pub fn extend(&mut self, gap: u32, tag: u32) -> Synth {
        let height = self.tip_height + 1;
        let time = self.times.last().unwrap() + gap;
        let window = self.rules.window(height);
        let newest_first =
            |v: &[u32], n: usize| v.iter().rev().take(n).copied().collect::<Vec<_>>();
        let bits = expected_bits(
            &self.rules,
            height,
            time,
            &newest_first(&self.times, window + MEDIAN_SPAN),
            &newest_first(&self.bits, window),
        )
        .expect("context is long enough");
        let mut merkle_root = [0u8; 32];
        merkle_root[..4].copy_from_slice(&tag.to_le_bytes());
        merkle_root[4..8].copy_from_slice(&height.to_le_bytes());
        merkle_root = header_bytes_hash(&merkle_root);
        let raw = mine(self.tip_hash, merkle_root, time, bits);
        let block = Synth {
            hash: header_bytes_hash(&raw),
            raw,
            height,
            merkle_root,
            bits,
        };
        self.push_known(block.clone(), time, bits);
        block
    }

    /// Like `extend` with the normal spacing.
    pub fn next(&mut self, tag: u32) -> Synth {
        self.extend(SPACING, tag)
    }
}

/// Grinds a nonce until the header hash meets `bits`.
fn mine(prev: [u8; 32], merkle_root: [u8; 32], time: u32, bits: u32) -> Vec<u8> {
    let mut raw = vec![0u8; HEADER_LEN];
    raw[0..4].copy_from_slice(&4u32.to_le_bytes());
    raw[4..36].copy_from_slice(&prev);
    raw[36..68].copy_from_slice(&merkle_root);
    raw[100..104].copy_from_slice(&time.to_le_bytes());
    raw[104..108].copy_from_slice(&bits.to_le_bytes());
    raw[140..143].copy_from_slice(&[0xfd, 0x40, 0x05]);
    for nonce in 0u64.. {
        raw[108..116].copy_from_slice(&nonce.to_le_bytes());
        if check_pow(Network::Testnet, bits, &header_bytes_hash(&raw)).is_ok() {
            return raw;
        }
    }
    unreachable!()
}

impl Harness {
    /// Uploads `header` and relays it with the Equihash state marked verified (see the module doc).
    /// Returns the compute units of `AcceptHeader` run alone, or its error code.
    pub fn relay_synthetic(&mut self, relay: &Address, header: &[u8]) -> Result<u64, u32> {
        let buffer = Self::buffer_addr(relay, &self.payer.pubkey(), 0);
        let split = 850;
        let create = [
            self.create_buffer_ix(relay, 0),
            self.write_ix(&buffer, 0, &header[..split]),
        ];
        self.send_code(&create)?;
        self.ok(&[self.write_ix(&buffer, split, &header[split..])], &[]);
        let mut account = self.svm.get_account(&buffer).unwrap();
        account.data[VERIFIER_OFFSET] = 2;
        self.svm.set_account(buffer, account).unwrap();
        let accept = [
            set_compute_limit(crate::CU_CAP),
            self.accept_ix(&buffer, relay),
        ];
        let result = self.send_code(&accept);
        self.ok(&[self.close_ix(&buffer)], &[]);
        result
    }

    /// Sends, returning the compute units used or the custom error code.
    fn send_code(&mut self, ixs: &[solana_instruction::Instruction]) -> Result<u64, u32> {
        match self.send(ixs, &[]) {
            Ok(meta) => Ok(meta.compute_units_consumed),
            Err(e) => match e.err {
                solana_transaction_error::TransactionError::InstructionError(
                    _,
                    solana_instruction::error::InstructionError::Custom(code),
                ) => Err(code),
                other => panic!("unexpected failure {other:?}\n{}", e.meta.pretty_logs()),
            },
        }
    }
}
