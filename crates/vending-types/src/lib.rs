//! Public-values integration stub. Fixed v1 wire format from the shared contract.
#![no_std]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;
pub const PUBLIC_VALUES_LEN: usize = 249;
#[cfg(test)]
mod tests {
    #[test]
    fn decode_checks_length_version_pool_and_reserved() {
        let mut b = [0; super::PUBLIC_VALUES_LEN];
        assert!(super::PublicValues::decode(&b).is_err());
        b[0] = 1;
        b[188] = 3;
        assert!(super::PublicValues::decode(&b).is_ok());
        assert!(super::PublicValues::decode(&b[..248]).is_err());
        b[248] = 1;
        assert!(super::PublicValues::decode(&b).is_err());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicValues {
    pub version: u8,
    pub branch_id: u32,
    pub machine: [u8; 32],
    pub nonce: [u8; 32],
    pub value: u64,
    pub payment_tag: [u8; 32],
    pub root: [u8; 32],
    pub height: u32,
    pub diversifier: [u8; 11],
    pub svk_hash: [u8; 32],
    pub pool: u8,
}
impl PublicValues {
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() != PUBLIC_VALUES_LEN
            || b[0] != 1
            || b[188] != 3
            || b[189..].iter().any(|v| *v != 0)
        {
            return Err(Error);
        }
        Ok(Self {
            version: b[0],
            branch_id: u32::from_le_bytes(b[1..5].try_into().map_err(|_| Error)?),
            machine: b[5..37].try_into().map_err(|_| Error)?,
            nonce: b[37..69].try_into().map_err(|_| Error)?,
            value: u64::from_le_bytes(b[69..77].try_into().map_err(|_| Error)?),
            payment_tag: b[77..109].try_into().map_err(|_| Error)?,
            root: b[109..141].try_into().map_err(|_| Error)?,
            height: u32::from_le_bytes(b[141..145].try_into().map_err(|_| Error)?),
            diversifier: b[145..156].try_into().map_err(|_| Error)?,
            svk_hash: b[156..188].try_into().map_err(|_| Error)?,
            pool: b[188],
        })
    }
}
