//! Difficulty adjustment and median-time rules, as Zcash consensus defines them (protocol spec
//! "Difficulty adjustment" and the block header `nTime` rules, ZIP-218 for the NU7 window).
//!
//! The functions take the candidate's context as slices, newest block first (`[0]` is the
//! parent), so the same code runs in the Header Relay and in off-chain replays.

use crate::pow::{target_from_compact, Network, U256};

/// Blocks of context a candidate needs: the NU7 averaging window plus the median span.
pub const CONTEXT_LEN: usize = 113;
/// Blocks in the median-time-past.
pub const MEDIAN_SPAN: usize = 11;
/// The NU7 averaging window, and the one before it.
pub const NU7_WINDOW: usize = 102;
pub const PRE_NU7_WINDOW: usize = 17;
const NU7_SPACING: u64 = 25;
const PRE_NU7_SPACING: u64 = 75;
/// A testnet block later than this many seconds after its parent may use the minimum difficulty
/// (6 x 75 s before NU7, 18 x 25 s after: the same 450 s).
pub const MIN_DIFFICULTY_GAP: i64 = 450;
const TESTNET_MIN_DIFFICULTY_HEIGHT: u32 = 299_188;
/// Longest a block's time may lead the median-time-past, in seconds.
pub const MAX_TIME_LEAD: u64 = 90 * 60;
const MAINNET_TIME_LEAD_HEIGHT: u32 = 2;
const TESTNET_TIME_LEAD_HEIGHT: u32 = 653_606;

/// What decides the rules for one Header Relay: the network and the height NU7 activates at
/// (`u32::MAX` when it is not scheduled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub network: Network,
    pub nu7_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DifficultyError {
    /// Fewer context blocks than the window plus the median span.
    ContextTooShort,
    /// A context block's `nBits` is not a valid target.
    InvalidContextBits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// `nTime` is not strictly after the median-time-past.
    NotAfterMedian,
    /// `nTime` is more than 90 minutes after the median-time-past.
    TooFarAhead,
    ContextTooShort,
}

impl Rules {
    /// Blocks averaged for a candidate at `height`: chosen by the candidate's height.
    pub fn window(&self, height: u32) -> usize {
        if height >= self.nu7_height {
            NU7_WINDOW
        } else {
            PRE_NU7_WINDOW
        }
    }

    fn spacing(&self, height: u32) -> u64 {
        if height >= self.nu7_height {
            NU7_SPACING
        } else {
            PRE_NU7_SPACING
        }
    }
}

/// Median of the first `MEDIAN_SPAN` times of `times` (fewer if there are fewer), the
/// median-time-past of the block that follows them.
fn median(times: &[u32]) -> u32 {
    let mut sorted = [0u32; MEDIAN_SPAN];
    let n = times.len().min(MEDIAN_SPAN);
    sorted[..n].copy_from_slice(&times[..n]);
    sorted[..n].sort_unstable();
    sorted[n / 2]
}

/// The `nBits` a block at `height` with time `time` must carry. `times` and `bits` hold the
/// context newest first; `times` needs window + 11 entries, `bits` the window.
pub fn expected_bits(
    rules: &Rules,
    height: u32,
    time: u32,
    times: &[u32],
    bits: &[u32],
) -> Result<u32, DifficultyError> {
    let window = rules.window(height);
    if times.len() < window + MEDIAN_SPAN || bits.len() < window {
        return Err(DifficultyError::ContextTooShort);
    }
    let limit = target_from_compact(rules.network.pow_limit_bits())
        .ok_or(DifficultyError::InvalidContextBits)?;

    if rules.network == Network::Testnet
        && height >= TESTNET_MIN_DIFFICULTY_HEIGHT
        && time as i64 - times[0] as i64 > MIN_DIFFICULTY_GAP
    {
        return Ok(limit.to_compact());
    }

    // floor(sum / window). A sum of 102 testnet limits needs 258 bits, so add into five limbs.
    let w = window as u64;
    let mut sum = [0u64; 5];
    for b in &bits[..window] {
        let target = target_from_compact(*b).ok_or(DifficultyError::InvalidContextBits)?;
        let mut carry = false;
        for (i, limb) in sum.iter_mut().enumerate() {
            let add = target.0.get(i).copied().unwrap_or(0);
            let (v, c1) = limb.overflowing_add(add);
            let (v, c2) = v.overflowing_add(carry as u64);
            *limb = v;
            carry = c1 || c2;
        }
    }
    let mut mean = [0u64; 5];
    let mut rem = 0u128;
    for i in (0..5).rev() {
        let v = (rem << 64) | sum[i] as u128;
        mean[i] = (v / w as u128) as u64;
        rem = v % w as u128;
    }
    if mean[4] != 0 {
        return Err(DifficultyError::InvalidContextBits);
    }
    let mean = U256([mean[0], mean[1], mean[2], mean[3]]);

    let span = w * rules.spacing(height);
    let actual = median(times) as i64 - median(&times[window..]) as i64;
    // Damp by 4, truncating toward zero, then clamp to -16% / +32% of the target timespan.
    let damped = span as i64 + (actual - span as i64) / 4;
    let bounded = damped.clamp((span * 84 / 100) as i64, (span * 132 / 100) as i64) as u64;

    let (per_second, _) = mean.div_rem_small(span);
    let threshold = per_second.checked_mul_small(bounded);
    let target = match threshold {
        Some(t) if t.cmp_to(&limit) != core::cmp::Ordering::Greater => t,
        _ => limit,
    };
    Ok(target.to_compact())
}

/// Checks `nTime` against the median-time-past of the previous eleven blocks (`times` newest first).
pub fn check_time(rules: &Rules, height: u32, time: u32, times: &[u32]) -> Result<(), TimeError> {
    if times.len() < MEDIAN_SPAN {
        return Err(TimeError::ContextTooShort);
    }
    let mtp = median(times);
    if time <= mtp {
        return Err(TimeError::NotAfterMedian);
    }
    let lead_from = match rules.network {
        Network::Mainnet => MAINNET_TIME_LEAD_HEIGHT,
        Network::Testnet => TESTNET_TIME_LEAD_HEIGHT,
    };
    if height >= lead_from && time as u64 > mtp as u64 + MAX_TIME_LEAD {
        return Err(TimeError::TooFarAhead);
    }
    Ok(())
}
