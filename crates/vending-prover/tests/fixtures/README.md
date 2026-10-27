# Machine revision 2 fixture

`machine-revision2.bin` is exactly 291 bytes, serialized with the program
worker's `programs/vending/src/state.rs` `Machine::write`, then round-tripped
with that producer's `Machine::read`. The consumer tests load these fixed
bytes directly, including through a mocked `getAccountInfo` response.

Producer commit: `54bfbb07c42850d69820151a24c3d9c9cc29af89`.

Producer source SHA-256:
`0bf1e3b28046a0225da0dfb36cdf3c2fe376107d844e8942a934e816a8ff7580`.

Fixture fields: discriminator/version 1/1; bumps 254/253; Payee `11`, seed `22`,
mint `33` (each repeated 32 times); Relay
`C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS`; price 100,000 zat;
available 7; stock 10; diversifier `6f78f490a60243c11a9fad`; public incoming
viewing key from `vending-guest/fixtures/shop.ivk.hex` and its SHA-256;
fixture vkey hash `44` repeated 32 times; reservation window 144 blocks.
This is synthetic configuration with a public incoming viewing key, not a
record of a deployed Machine. It contains no spending or full viewing key.
