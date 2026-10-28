//! Tip following: ask a source for the headers after what the relay already has, submit them in
//! order, and keep going after failures. The relay on Solana is the only state that matters; a
//! restart rebuilds everything from it.
use std::fmt;

use crate::p2p::{sha256d, RawHeader};

pub type Hash = [u8; 32];

/// The Store holds at most 400 headers.
const MAX_KNOWN: usize = 512;
/// Newest hashes a locator carries one by one before it starts skipping.
const DENSE_LOCATOR: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tip {
    pub height: u32,
    pub hash: Hash,
    /// 0 while the relay runs; otherwise the Halt reason.
    pub halted: u8,
}

/// Progress inside `RelayChain::submit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Uploaded,
    Verified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The relay accepted the header; `tip` is the relay's tip afterwards.
    Accepted { tip: Tip, signatures: Vec<String> },
    /// The relay already holds this header.
    AlreadyKnown,
    /// The relay refused the header; resubmitting the same bytes cannot help.
    Rejected { code: u32, reason: String },
}

#[derive(Debug)]
pub enum ChainError {
    /// Network, RPC or confirmation trouble: the same call may succeed later.
    Transient(String),
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Transient(m) = self;
        write!(f, "{m}")
    }
}

pub trait RelayChain {
    fn tip(&mut self) -> Result<Tip, ChainError>;
    /// Hashes and heights of the blocks the relay holds, newest first.
    fn known_blocks(&mut self) -> Result<Vec<(Hash, u32)>, ChainError>;
    fn submit(
        &mut self,
        header: &RawHeader,
        stage: &mut dyn FnMut(Stage),
    ) -> Result<Outcome, ChainError>;
}

pub struct Batch {
    pub peer: String,
    pub headers: Vec<RawHeader>,
}

pub trait HeaderSource {
    /// Headers that follow the first locator hash a peer knows; empty when the peer has nothing newer.
    fn headers_after(&mut self, locator: &[Hash]) -> Result<Batch, String>;
    /// The peer sent headers the relay could not use: prefer others.
    fn penalize(&mut self, peer: &str);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Tip {
        height: u32,
        hash: Hash,
    },
    Uploaded {
        height: u32,
        hash: Hash,
    },
    Verified {
        height: u32,
        hash: Hash,
    },
    Accepted {
        height: u32,
        hash: Hash,
        tip_height: u32,
        signatures: Vec<String>,
    },
    Known {
        height: u32,
        hash: Hash,
    },
    Rejected {
        height: u32,
        hash: Hash,
        code: u32,
        reason: String,
        peer: String,
    },
    Unlinked {
        peer: String,
    },
    Halted {
        reason: u8,
    },
    Warn(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// The relay is Halted; nothing is submitted.
    Halted,
    /// The source had nothing new.
    Idle,
    /// Headers went in (accepted or already known); more may follow.
    Progress { accepted: usize },
    /// A header was refused; the rest of the batch was dropped.
    Rejected,
}

pub struct Follower<C, S, E> {
    chain: C,
    source: S,
    events: E,
    /// Blocks the relay holds, newest first.
    known: Vec<(Hash, u32)>,
}

impl<C: RelayChain, S: HeaderSource, E: FnMut(Event)> Follower<C, S, E> {
    pub fn new(chain: C, source: S, events: E) -> Self {
        Self {
            chain,
            source,
            events,
            known: Vec::new(),
        }
    }

    pub fn chain_mut(&mut self) -> &mut C {
        &mut self.chain
    }

    fn remember(&mut self, hash: Hash, height: u32) {
        if self.known.iter().any(|(h, _)| *h == hash) {
            return;
        }
        let at = self.known.partition_point(|(_, h)| *h >= height);
        self.known.insert(at, (hash, height));
        self.known.truncate(MAX_KNOWN);
    }

    /// Newest first: the tip, then every block for a while, then ever larger steps, so a fork point
    /// anywhere in the relay's window is covered by about forty hashes.
    fn locator(&self, tip: &Tip) -> Vec<Hash> {
        let mut out = vec![tip.hash];
        let (mut step, mut index) = (1, 0);
        while index < self.known.len() {
            let hash = self.known[index].0;
            if hash != tip.hash {
                out.push(hash);
            }
            if index >= DENSE_LOCATOR {
                step *= 2;
            }
            index += step;
        }
        if let Some((oldest, _)) = self.known.last() {
            if !out.contains(oldest) {
                out.push(*oldest);
            }
        }
        out
    }

    fn height_of(&self, hash: &Hash) -> Option<u32> {
        self.known.iter().find(|(h, _)| h == hash).map(|(_, h)| *h)
    }

    /// One poll: read the relay's tip, fetch what follows it, submit it.
    pub fn step(&mut self) -> Result<Step, ChainError> {
        let tip = self.chain.tip()?;
        if tip.halted != 0 {
            (self.events)(Event::Halted { reason: tip.halted });
            return Ok(Step::Halted);
        }
        if self.known.is_empty() {
            self.known = self.chain.known_blocks()?;
        }
        self.remember(tip.hash, tip.height);
        let batch = match self.source.headers_after(&self.locator(&tip)) {
            Ok(batch) => batch,
            Err(e) => {
                (self.events)(Event::Warn(format!("no peer answered: {e}")));
                return Ok(Step::Idle);
            }
        };
        let Some(first) = batch.headers.first() else {
            return Ok(Step::Idle);
        };
        let first_parent: Hash = first[4..36].try_into().unwrap();
        let Some(mut height) = self.height_of(&first_parent) else {
            self.source.penalize(&batch.peer);
            (self.events)(Event::Unlinked { peer: batch.peer });
            // The relay's tip may sit on a branch the peers dropped: next time, use the Store's blocks.
            self.known = self.chain.known_blocks()?;
            return Ok(Step::Idle);
        };

        let mut parent = first_parent;
        let mut accepted = 0;
        let mut submitted = 0;
        for header in &batch.headers {
            if header[4..36] != parent {
                self.source.penalize(&batch.peer);
                (self.events)(Event::Unlinked {
                    peer: batch.peer.clone(),
                });
                break;
            }
            height += 1;
            let hash = sha256d(header);
            parent = hash;
            if self.height_of(&hash).is_some() {
                // The relay already holds it; submitting would cost fees to learn that.
                continue;
            }
            submitted += 1;
            let mut events = Vec::new();
            let outcome = self.chain.submit(header, &mut |s| events.push(s))?;
            for stage in events {
                (self.events)(match stage {
                    Stage::Uploaded => Event::Uploaded { height, hash },
                    Stage::Verified => Event::Verified { height, hash },
                });
            }
            match outcome {
                Outcome::Accepted { tip, signatures } => {
                    self.remember(hash, height);
                    accepted += 1;
                    (self.events)(Event::Accepted {
                        height,
                        hash,
                        tip_height: tip.height,
                        signatures,
                    });
                    (self.events)(Event::Tip {
                        height: tip.height,
                        hash: tip.hash,
                    });
                    if tip.halted != 0 {
                        (self.events)(Event::Halted { reason: tip.halted });
                        return Ok(Step::Halted);
                    }
                }
                Outcome::AlreadyKnown => {
                    self.remember(hash, height);
                    (self.events)(Event::Known { height, hash });
                }
                Outcome::Rejected { code, reason } => {
                    self.source.penalize(&batch.peer);
                    (self.events)(Event::Rejected {
                        height,
                        hash,
                        code,
                        reason,
                        peer: batch.peer,
                    });
                    return Ok(Step::Rejected);
                }
            }
        }
        if submitted == 0 {
            return Ok(Step::Idle);
        }
        Ok(Step::Progress { accepted })
    }
}
