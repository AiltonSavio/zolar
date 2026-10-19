/// Custom program error codes, in the order of `clients/codama/escrow.mjs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum EscrowError {
    /// An account is not the one the Escrow expects.
    WrongAccount = 1,
    /// Expiry is not in (relay tip, relay tip + 20].
    BadExpiry = 2,
    ZeroAmount = 3,
    /// Commitment or salt is not below the BN254 modulus.
    CommitmentOutOfField = 4,
    /// A token account has the wrong mint, owner, state, delegate or balance.
    BadTokenAccount = 5,
    NotOpen = 6,
    RelayHalted = 7,
    CommitmentMismatch = 8,
    /// The block is not above the relay tip at creation, or is above the expiry height.
    BlockOutOfRange = 9,
    /// The transaction Merkle path is longer than the limit.
    BadInclusionProof = 10,
    /// The Final Block Tree path does not lead to a recent root that already holds the block.
    UnknownRoot = 11,
    /// The relay account is not a sealed Header Relay.
    RelayNotReadable = 12,
    PoseidonFailed = 13,
    /// A Bond is not supported yet (ticket 09).
    BondNotSupported = 14,
    /// The claim mode is not supported, or the Escrow does not allow a Reveal Claim.
    ClaimModeNotAllowed = 15,
    /// The Groth16 proof does not verify against the Escrow's public inputs.
    InvalidProof = 16,
}
