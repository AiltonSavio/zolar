//! Fixed-width account layouts. Integers are little endian, no padding.
pub const VERSION: u8 = 1;
pub const MACHINE_LEN: usize = 227;
pub const RESERVATION_LEN: usize = 119;
pub const TAG_LEN: usize = 2;
pub const BACKSTOP_SECS: i64 = 86_400;
pub const NU7_BRANCH_ID: u32 = 0x7719_0ad9;
pub const TOKEN_PROGRAM: [u8; 32] = [
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133, 237,
    95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    pub bump: u8,
    pub mint_bump: u8,
    pub payee: [u8; 32],
    pub seed: [u8; 32],
    pub mint: [u8; 32],
    pub relay: [u8; 32],
    pub price_zat: u64,
    pub available: u32,
    pub stock: u32,
    pub diversifier: [u8; 11],
    pub svk_hash: [u8; 32],
    pub vkey_hash: [u8; 32],
    pub reservation_blocks: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub bump: u8,
    pub machine: [u8; 32],
    pub nonce: [u8; 32],
    pub payout: [u8; 32],
    pub price_zat: u64,
    pub expiry_height: u32,
    pub created_at: i64,
}

fn array<const N: usize>(d: &[u8], at: usize) -> [u8; N] {
    d[at..at + N].try_into().expect("validated fixed layout")
}
impl Machine {
    pub fn read(d: &[u8]) -> Option<Self> {
        if d.len() != MACHINE_LEN || d[0] != 1 || d[1] != VERSION {
            return None;
        }
        Some(Self {
            bump: d[2],
            mint_bump: d[3],
            payee: array(d, 4),
            seed: array(d, 36),
            mint: array(d, 68),
            relay: array(d, 100),
            price_zat: u64::from_le_bytes(array(d, 132)),
            available: u32::from_le_bytes(array(d, 140)),
            stock: u32::from_le_bytes(array(d, 144)),
            diversifier: array(d, 148),
            svk_hash: array(d, 159),
            vkey_hash: array(d, 191),
            reservation_blocks: u32::from_le_bytes(array(d, 223)),
        })
    }
    pub fn write(&self, d: &mut [u8]) {
        let fields: &[&[u8]] = &[
            &[1, VERSION, self.bump, self.mint_bump],
            &self.payee,
            &self.seed,
            &self.mint,
            &self.relay,
            &self.price_zat.to_le_bytes(),
            &self.available.to_le_bytes(),
            &self.stock.to_le_bytes(),
            &self.diversifier,
            &self.svk_hash,
            &self.vkey_hash,
            &self.reservation_blocks.to_le_bytes(),
        ];
        let mut at = 0;
        for f in fields {
            d[at..at + f.len()].copy_from_slice(f);
            at += f.len();
        }
    }
}
impl Reservation {
    pub fn read(d: &[u8]) -> Option<Self> {
        if d.len() != RESERVATION_LEN || d[0] != 2 || d[1] != VERSION {
            return None;
        }
        Some(Self {
            bump: d[2],
            machine: array(d, 3),
            nonce: array(d, 35),
            payout: array(d, 67),
            price_zat: u64::from_le_bytes(array(d, 99)),
            expiry_height: u32::from_le_bytes(array(d, 107)),
            created_at: i64::from_le_bytes(array(d, 111)),
        })
    }
    pub fn write(&self, d: &mut [u8]) {
        let fields: &[&[u8]] = &[
            &[2, VERSION, self.bump],
            &self.machine,
            &self.nonce,
            &self.payout,
            &self.price_zat.to_le_bytes(),
            &self.expiry_height.to_le_bytes(),
            &self.created_at.to_le_bytes(),
        ];
        let mut at = 0;
        for f in fields {
            d[at..at + f.len()].copy_from_slice(f);
            at += f.len();
        }
    }
}
