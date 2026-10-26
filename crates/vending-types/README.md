# Vending types

Dependency-free `no_std` encoding shared by the SP1 guest and the Solana Vending Machine.

```rust
use vending_types::{Memo, PublicValues, DecodeError, PUBLIC_VALUES_LEN};
// PublicValues::encode(&self) -> [u8; 249]
// PublicValues::decode(bytes: &[u8]) -> Result<PublicValues, DecodeError>
// Memo::encode(&self) -> [u8; 512]
// Memo::decode(bytes: &[u8]) -> Result<Memo, DecodeError>
```

`PublicValues` fields, in order: `branch_id: u32`, `machine: [u8;32]`,
`nonce: [u8;32]`, `value_zat: u64`, `payment_tag: [u8;32]`, `root: [u8;32]`,
`height: u32`, `diversifier: [u8;11]`, `svk_hash: [u8;32]`.

The encoding adds version `1` at byte 0, pool `3` at byte 188, and 60 zero
reserved bytes at 189. Integers are little-endian. Hashes retain their protocol
byte order; the Final Block Tree root is a big-endian BN254 element. Decoding
rejects incorrect lengths, versions, pools and nonzero reserved bytes.

`Memo { machine, nonce }` encodes `ff 01 || machine || nonce || 446 zero bytes`.
Decoding requires exactly 512 bytes and rejects any other prefix or padding.
The on-chain Claim must still bind every field to its machine, Reservation and
running Header Relay; decoding alone is not a payment verification.
