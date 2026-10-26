//! Canonical vending v1 public values and ZIP-302 text memo.
//! Integers are little-endian; hashes retain their protocol byte order.
#![no_std]

pub const PUBLIC_VALUES_LEN: usize = 249;
pub const VERSION: u8 = 1;
pub const IRONWOOD_POOL: u8 = 3;
pub const MEMO_LEN: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Length,
    Version,
    Pool,
    Reserved,
    Memo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicValues {
    pub branch_id: u32,
    pub machine: [u8; 32],
    pub nonce: [u8; 32],
    pub value_zat: u64,
    pub payment_tag: [u8; 32],
    pub root: [u8; 32],
    pub height: u32,
    pub diversifier: [u8; 11],
    pub svk_hash: [u8; 32],
    pub payout: [u8; 32],
}

impl PublicValues {
    pub fn encode(&self) -> [u8; PUBLIC_VALUES_LEN] {
        let mut b = [0; PUBLIC_VALUES_LEN];
        b[0] = VERSION;
        b[1..5].copy_from_slice(&self.branch_id.to_le_bytes());
        b[5..37].copy_from_slice(&self.machine);
        b[37..69].copy_from_slice(&self.nonce);
        b[69..77].copy_from_slice(&self.value_zat.to_le_bytes());
        b[77..109].copy_from_slice(&self.payment_tag);
        b[109..141].copy_from_slice(&self.root);
        b[141..145].copy_from_slice(&self.height.to_le_bytes());
        b[145..156].copy_from_slice(&self.diversifier);
        b[156..188].copy_from_slice(&self.svk_hash);
        b[188] = IRONWOOD_POOL;
        b[189..221].copy_from_slice(&self.payout);
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        if b.len() != PUBLIC_VALUES_LEN {
            return Err(DecodeError::Length);
        }
        if b[0] != VERSION {
            return Err(DecodeError::Version);
        }
        if b[188] != IRONWOOD_POOL {
            return Err(DecodeError::Pool);
        }
        if b[221..].iter().any(|x| *x != 0) {
            return Err(DecodeError::Reserved);
        }
        Ok(Self {
            branch_id: u32::from_le_bytes(b[1..5].try_into().unwrap()),
            machine: b[5..37].try_into().unwrap(),
            nonce: b[37..69].try_into().unwrap(),
            value_zat: u64::from_le_bytes(b[69..77].try_into().unwrap()),
            payment_tag: b[77..109].try_into().unwrap(),
            root: b[109..141].try_into().unwrap(),
            height: u32::from_le_bytes(b[141..145].try_into().unwrap()),
            diversifier: b[145..156].try_into().unwrap(),
            svk_hash: b[156..188].try_into().unwrap(),
            payout: b[189..221].try_into().unwrap(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memo {
    pub machine: [u8; 32],
    pub nonce: [u8; 32],
    pub payout: [u8; 32],
}

impl Memo {
    pub fn encode(&self) -> [u8; MEMO_LEN] {
        let mut b = [0; MEMO_LEN];
        b[..4].copy_from_slice(b"zv1:");
        let mut at = 4;
        for (i, field) in [&self.machine, &self.nonce, &self.payout]
            .iter()
            .enumerate()
        {
            if i > 0 {
                b[at] = b':';
                at += 1;
            }
            at += bs58::encode(field)
                .onto(&mut b[at..])
                .expect("32-byte base58 fits memo");
        }
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        if b.len() != MEMO_LEN {
            return Err(DecodeError::Length);
        }
        let end = b.iter().position(|x| *x == 0).ok_or(DecodeError::Memo)?;
        if !b[..end].starts_with(b"zv1:") || b[end..].iter().any(|x| *x != 0) {
            return Err(DecodeError::Memo);
        }
        let mut fields = b[4..end].split(|x| *x == b':');
        let mut read = || -> Result<[u8; 32], DecodeError> {
            let field = fields.next().ok_or(DecodeError::Memo)?;
            let mut bytes = [0; 32];
            let len = bs58::decode(field)
                .onto(&mut bytes[..])
                .map_err(|_| DecodeError::Memo)?;
            if len != 32 {
                return Err(DecodeError::Memo);
            }
            let mut canonical = [0; 44];
            let len = bs58::encode(bytes)
                .onto(&mut canonical[..])
                .map_err(|_| DecodeError::Memo)?;
            if canonical[..len] != *field {
                return Err(DecodeError::Memo);
            }
            Ok(bytes)
        };
        let memo = Self {
            machine: read()?,
            nonce: read()?,
            payout: read()?,
        };
        if fields.next().is_some() {
            return Err(DecodeError::Memo);
        }
        Ok(memo)
    }
}
