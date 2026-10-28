# Vending types

`no_std` fixed-width encoding shared by the SP1 guest and the Solana Vending Machine.
Base58 uses `bs58` without its default features and only fixed-size stack buffers.

```rust
use vending_types::{Memo, PublicValues, DecodeError, PUBLIC_VALUES_LEN};
// PublicValues::encode(&self) -> [u8; 249]
// PublicValues::decode(bytes: &[u8]) -> Result<PublicValues, DecodeError>
// Memo::encode(&self) -> [u8; 512]
// Memo::decode(bytes: &[u8]) -> Result<Memo, DecodeError>
```

`PublicValues` fields: `branch_id: u32`, `machine: [u8;32]`, `nonce: [u8;32]`,
`value_zat: u64`, `payment_tag: [u8;32]`, `root: [u8;32]`, `height: u32`,
`diversifier: [u8;11]`, `svk_hash: [u8;32]`, `payout: [u8;32]`.

The revision 2 encoding has version `1` at byte 0, pool `3` at byte 188,
`payout` at `189..221`, and 28 zero reserved bytes at `221..249`. All earlier
offsets are unchanged. Integers are little-endian; the Final Block Tree root
is a big-endian BN254 element. Decoding rejects incorrect lengths, versions,
pools and nonzero reserved bytes.

`Memo { machine, nonce, payout }` encodes exactly the ASCII text
`zv1:<machine base58>:<nonce base58>:<payout base58>`, zero-padded to 512 bytes.
Each field must canonically encode exactly 32 bytes. Decoding rejects binary
memos, uppercase prefixes, spaces, extra fields, invalid or wrong-width
base58, embedded NULs and nonzero padding. The on-chain Claim must bind every
public value to the Machine, Reservation and Header Relay; decoding alone
is not payment verification.
