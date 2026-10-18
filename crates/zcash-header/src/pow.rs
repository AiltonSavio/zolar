//! Compact targets, proof-of-work checks and chain work.

use crate::header::{self, Sha256, HEADER_LEN};

/// 256-bit unsigned integer, four little-endian 64-bit limbs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct U256(pub [u64; 4]);

impl U256 {
    pub const ZERO: U256 = U256([0; 4]);

    /// Reads 32 little-endian bytes.
    pub fn from_le_bytes(bytes: &[u8; 32]) -> Self {
        let mut limbs = [0u64; 4];
        for (limb, chunk) in limbs.iter_mut().zip(bytes.chunks_exact(8)) {
            let mut b = [0u8; 8];
            b.copy_from_slice(chunk);
            *limb = u64::from_le_bytes(b);
        }
        U256(limbs)
    }

    fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    fn cmp_to(&self, other: &Self) -> core::cmp::Ordering {
        for i in (0..4).rev() {
            match self.0[i].cmp(&other.0[i]) {
                core::cmp::Ordering::Equal => {}
                unequal => return unequal,
            }
        }
        core::cmp::Ordering::Equal
    }

    fn shl_bytes(self, bytes: usize) -> Option<Self> {
        let mut out = [0u8; 32];
        let mut src = [0u8; 32];
        for (i, limb) in self.0.iter().enumerate() {
            src[i * 8..i * 8 + 8].copy_from_slice(&limb.to_le_bytes());
        }
        for (i, b) in src.iter().enumerate() {
            if *b == 0 {
                continue;
            }
            *out.get_mut(i + bytes)? = *b;
        }
        Some(Self::from_le_bytes(&out))
    }

    fn shl1(&mut self) {
        let mut carry = 0;
        for limb in self.0.iter_mut() {
            let next = *limb >> 63;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
    }

    fn sub_assign(&mut self, other: &Self) {
        let mut borrow = false;
        for i in 0..4 {
            let (d, b1) = self.0[i].overflowing_sub(other.0[i]);
            let (d, b2) = d.overflowing_sub(borrow as u64);
            self.0[i] = d;
            borrow = b1 || b2;
        }
    }

    fn bit(&self, i: usize) -> bool {
        (self.0[i / 64] >> (i % 64)) & 1 == 1
    }

    /// `(self / divisor, remainder)`; `divisor` must not be zero.
    fn div_rem(self, divisor: &Self) -> (Self, Self) {
        let mut quotient = Self::ZERO;
        let mut rem = Self::ZERO;
        for i in (0..256).rev() {
            rem.shl1();
            rem.0[0] |= self.bit(i) as u64;
            if rem.cmp_to(divisor) != core::cmp::Ordering::Less {
                rem.sub_assign(divisor);
                quotient.0[i / 64] |= 1 << (i % 64);
            }
        }
        (quotient, rem)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    /// Compact form of the network's proof-of-work limit (the easiest allowed target).
    pub const fn pow_limit_bits(self) -> u32 {
        match self {
            Network::Mainnet => 0x1f07ffff,
            Network::Testnet => 0x2007ffff,
        }
    }
}

/// Expands a compact `nBits` into a target. `None` for a negative, zero or
/// overflowing encoding, which consensus rejects.
pub fn target_from_compact(bits: u32) -> Option<U256> {
    let size = (bits >> 24) as usize;
    let mantissa = bits & 0x007f_ffff;
    if mantissa != 0 && bits & 0x0080_0000 != 0 {
        return None;
    }
    let target = if size <= 3 {
        U256([(mantissa >> (8 * (3 - size))) as u64, 0, 0, 0])
    } else {
        U256([mantissa as u64, 0, 0, 0]).shl_bytes(size - 3)?
    };
    (!target.is_zero()).then_some(target)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowError {
    /// `nBits` is negative, zero or overflows.
    InvalidBits,
    /// The target is easier than the network's proof-of-work limit.
    AboveLimit,
    /// The header hash, read as a little-endian number, exceeds the target.
    HashAboveTarget,
    /// The target is so small that its work does not fit in 128 bits.
    WorkOverflow,
}

/// Checks `nBits` is a valid target within the network limit and that the header
/// hash meets it. Returns the target.
pub fn check_pow(network: Network, bits: u32, hash: &[u8; 32]) -> Result<U256, PowError> {
    let target = target_from_compact(bits).ok_or(PowError::InvalidBits)?;
    let limit = target_from_compact(network.pow_limit_bits()).ok_or(PowError::InvalidBits)?;
    if target.cmp_to(&limit) == core::cmp::Ordering::Greater {
        return Err(PowError::AboveLimit);
    }
    if U256::from_le_bytes(hash).cmp_to(&target) == core::cmp::Ordering::Greater {
        return Err(PowError::HashAboveTarget);
    }
    Ok(target)
}

/// Expected number of hashes behind a block with this target,
/// `floor(2^256 / (target + 1))`. `None` when that does not fit in 128 bits (a target below
/// `2^128`, far harder than any real block).
pub fn work(target: &U256) -> Option<u128> {
    // (2^256 - 1 - target) / (target + 1) + 1, which avoids a 257-bit numerator.
    let mut numerator = U256([u64::MAX; 4]);
    numerator.sub_assign(target);
    let mut divisor = *target;
    let mut carry = true;
    for limb in divisor.0.iter_mut() {
        let (v, c) = limb.overflowing_add(carry as u64);
        *limb = v;
        carry = c;
    }
    if carry {
        return Some(1);
    }
    let (q, _) = numerator.div_rem(&divisor);
    if q.0[2] != 0 || q.0[3] != 0 {
        return None;
    }
    (q.0[0] as u128 | (q.0[1] as u128) << 64).checked_add(1)
}

/// Checks a serialised header's proof-of-work target and returns its chain work.
pub fn header_work(
    sha256: Sha256,
    network: Network,
    header: &[u8; HEADER_LEN],
) -> Result<u128, PowError> {
    let hash = header::hash(sha256, header);
    let target = check_pow(network, header::bits(header), &hash)?;
    work(&target).ok_or(PowError::WorkOverflow)
}
