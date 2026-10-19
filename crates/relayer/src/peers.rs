//! A pool of live testnet peers as a header source. Connections are kept open, peers that fail are
//! dropped and rested, and peers whose headers the relay could not use are avoided.
use std::{
    collections::HashMap,
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

use rand::seq::SliceRandom;

use crate::{
    follower::{Batch, Hash, HeaderSource},
    p2p::{self, Peer},
};

/// Live connections to keep.
const TARGET_PEERS: usize = 3;
/// Connection attempts per request before giving up.
const CONNECT_ATTEMPTS: usize = 8;
const REST_AFTER_FAILURE: Duration = Duration::from_secs(300);
const BAN: Duration = Duration::from_secs(3600);
const STRIKES_FOR_BAN: u32 = 2;
const REFRESH_SEEDS: Duration = Duration::from_secs(900);

struct Live {
    addr: SocketAddr,
    peer: Peer<TcpStream>,
    strikes: u32,
}

pub struct PeerPool {
    hosts: Vec<String>,
    candidates: Vec<SocketAddr>,
    resolved_at: Option<Instant>,
    live: Vec<Live>,
    resting: HashMap<SocketAddr, Instant>,
    next: usize,
}

impl PeerPool {
    pub fn new(hosts: Vec<String>) -> Self {
        Self {
            hosts,
            candidates: Vec::new(),
            resolved_at: None,
            live: Vec::new(),
            resting: HashMap::new(),
            next: 0,
        }
    }

    fn rest(&mut self, addr: SocketAddr, for_how_long: Duration) {
        self.resting.insert(addr, Instant::now() + for_how_long);
    }

    fn refresh_candidates(&mut self) {
        if self
            .resolved_at
            .is_some_and(|t| t.elapsed() < REFRESH_SEEDS)
            && !self.candidates.is_empty()
        {
            return;
        }
        let mut found = p2p::resolve(&self.hosts);
        found.sort();
        found.dedup();
        self.candidates = found;
        self.resolved_at = Some(Instant::now());
    }

    fn fill(&mut self) {
        self.refresh_candidates();
        let now = Instant::now();
        self.resting.retain(|_, until| *until > now);
        let mut free: Vec<SocketAddr> = self
            .candidates
            .iter()
            .copied()
            .filter(|a| !self.resting.contains_key(a) && self.live.iter().all(|l| l.addr != *a))
            .collect();
        free.shuffle(&mut rand::thread_rng());
        for addr in free.into_iter().take(CONNECT_ATTEMPTS) {
            if self.live.len() >= TARGET_PEERS {
                break;
            }
            match Peer::connect(addr) {
                Ok(peer) => self.live.push(Live {
                    addr,
                    peer,
                    strikes: 0,
                }),
                Err(_) => self.rest(addr, REST_AFTER_FAILURE),
            }
        }
    }
}

impl HeaderSource for PeerPool {
    fn headers_after(&mut self, locator: &[Hash]) -> Result<Batch, String> {
        if self.live.len() < TARGET_PEERS {
            self.fill();
        }
        if self.live.is_empty() {
            return Err("no peer connected".into());
        }
        let mut last_error = String::new();
        for _ in 0..self.live.len() {
            self.next = (self.next + 1) % self.live.len();
            let live = &mut self.live[self.next];
            match live.peer.getheaders(locator) {
                Ok(headers) => {
                    return Ok(Batch {
                        peer: live.addr.to_string(),
                        headers,
                    })
                }
                Err(e) => {
                    last_error = format!("{}: {e}", live.addr);
                    let addr = live.addr;
                    self.live.remove(self.next);
                    self.rest(addr, REST_AFTER_FAILURE);
                    // Indexes shifted; try again from the same position.
                    self.next = self.next.saturating_sub(1);
                    if self.live.is_empty() {
                        break;
                    }
                }
            }
        }
        Err(last_error)
    }

    fn penalize(&mut self, peer: &str) {
        let Some(i) = self.live.iter().position(|l| l.addr.to_string() == peer) else {
            return;
        };
        self.live[i].strikes += 1;
        if self.live[i].strikes >= STRIKES_FOR_BAN {
            let addr = self.live.remove(i).addr;
            self.rest(addr, BAN);
        }
    }
}
