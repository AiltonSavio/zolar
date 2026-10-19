//! A pool of live peers of one Zcash network as a header source. Connections are kept open, peers that fail are
//! dropped and rested, and peers whose headers the relay could not use are avoided.
use std::{
    collections::HashMap,
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

use rand::seq::SliceRandom;

use crate::{
    follower::{Batch, Hash, HeaderSource},
    p2p::{self, Network, Peer},
};

/// Live connections to keep.
const TARGET_PEERS: usize = 3;
/// Connection attempts per request before giving up.
const CONNECT_ATTEMPTS: usize = 8;
const REST_AFTER_FAILURE: Duration = Duration::from_secs(300);
const BAN: Duration = Duration::from_secs(3600);
const STRIKES_FOR_BAN: u32 = 2;
const REFRESH_SEEDS: Duration = Duration::from_secs(900);
/// Peers can stay connected yet stop following the chain; replace each one regularly. Public
/// peers (mainnet above all) often refuse a new connection, so an old one is only retired once a
/// replacement is connected or it fails.
const DEFAULT_ROTATE_AFTER: Duration = Duration::from_secs(900);

struct Live {
    addr: SocketAddr,
    peer: Peer<TcpStream>,
    strikes: u32,
    since: Instant,
}

pub struct PeerPool {
    network: Network,
    hosts: Vec<String>,
    candidates: Vec<SocketAddr>,
    resolved_at: Option<Instant>,
    live: Vec<Live>,
    resting: HashMap<SocketAddr, Instant>,
    banned: HashMap<SocketAddr, Instant>,
    rotate_after: Duration,
    next: usize,
}

impl PeerPool {
    pub fn new(hosts: Vec<String>, network: Network) -> Self {
        Self {
            network,
            hosts,
            candidates: Vec::new(),
            resolved_at: None,
            live: Vec::new(),
            resting: HashMap::new(),
            banned: HashMap::new(),
            rotate_after: DEFAULT_ROTATE_AFTER,
            next: 0,
        }
    }

    /// Replaces connections older than `after` (tests use zero).
    pub fn rotating_after(mut self, after: Duration) -> Self {
        self.rotate_after = after;
        self
    }

    fn is_stale(&self, live: &Live) -> bool {
        live.since.elapsed() >= self.rotate_after
    }

    fn fresh_count(&self) -> usize {
        self.live.iter().filter(|l| !self.is_stale(l)).count()
    }

    fn rest(&mut self, addr: SocketAddr, for_how_long: Duration) {
        self.resting.insert(addr, Instant::now() + for_how_long);
    }

    fn ban(&mut self, addr: SocketAddr) {
        self.banned.insert(addr, Instant::now() + BAN);
    }

    fn refresh_candidates(&mut self) {
        if self
            .resolved_at
            .is_some_and(|t| t.elapsed() < REFRESH_SEEDS)
            && !self.candidates.is_empty()
        {
            return;
        }
        let mut found = p2p::resolve(&self.hosts, self.network);
        found.sort();
        found.dedup();
        self.candidates = found;
        self.resolved_at = Some(Instant::now());
    }

    fn free_candidates(&self) -> Vec<SocketAddr> {
        self.candidates
            .iter()
            .copied()
            .filter(|a| {
                !self.resting.contains_key(a)
                    && !self.banned.contains_key(a)
                    && self.live.iter().all(|l| l.addr != *a)
            })
            .collect()
    }

    fn fill(&mut self) {
        self.refresh_candidates();
        let now = Instant::now();
        self.resting.retain(|_, until| *until > now);
        self.banned.retain(|_, until| *until > now);
        let mut free = self.free_candidates();
        if free.is_empty() && self.live.is_empty() {
            // Every peer failed recently (public peers refuse and time out often): try them again
            // rather than leaving the Relayer without a source. Banned peers stay banned.
            self.resting.clear();
            free = self.free_candidates();
        }
        free.shuffle(&mut rand::thread_rng());
        for addr in free.into_iter().take(CONNECT_ATTEMPTS) {
            if self.fresh_count() >= TARGET_PEERS {
                break;
            }
            match Peer::connect(addr, self.network) {
                Ok(peer) => self.live.push(Live {
                    addr,
                    peer,
                    strikes: 0,
                    since: Instant::now(),
                }),
                Err(_) => self.rest(addr, REST_AFTER_FAILURE),
            }
        }
    }
}

impl HeaderSource for PeerPool {
    fn headers_after(&mut self, locator: &[Hash]) -> Result<Batch, String> {
        if self.fresh_count() < TARGET_PEERS {
            self.fill();
        }
        if self.fresh_count() > 0 {
            let rotate_after = self.rotate_after;
            self.live.retain(|l| l.since.elapsed() < rotate_after);
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
            self.ban(addr);
        }
    }
}
