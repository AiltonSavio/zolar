use zcash_header::{equihash, pow::PowError};

/// Custom program error codes. 1 to 7 are the Equihash verifier's, in its own order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RelayError {
    MalformedHeader = 1,
    MalformedState = 2,
    WrongStage = 3,
    Collision = 4,
    OutOfOrder = 5,
    DuplicateIndex = 6,
    NonZeroRoot = 7,
    /// The header changed after verification began.
    HeaderFrozen = 8,
    InvalidBits = 9,
    TargetAboveLimit = 10,
    HashAboveTarget = 11,
    UnknownParent = 12,
    DuplicateHeader = 13,
    NotVerified = 14,
    RelayNotActive = 15,
    RelayHalted = 16,
    CheckpointDigestMismatch = 17,
    InvalidCheckpoint = 18,
    WrongAccount = 19,
    AlreadyAccepted = 20,
    StoreNotGrowable = 21,
    RelayAlreadySealed = 22,
    /// The next slot to overwrite holds the current tip.
    StoreFull = 23,
}

impl From<equihash::Error> for RelayError {
    fn from(e: equihash::Error) -> Self {
        use equihash::Error::*;
        match e {
            MalformedHeader => Self::MalformedHeader,
            MalformedState => Self::MalformedState,
            WrongStage => Self::WrongStage,
            Collision => Self::Collision,
            OutOfOrder => Self::OutOfOrder,
            DuplicateIndex => Self::DuplicateIndex,
            NonZeroRoot => Self::NonZeroRoot,
        }
    }
}

impl From<PowError> for RelayError {
    fn from(e: PowError) -> Self {
        match e {
            PowError::InvalidBits => Self::InvalidBits,
            PowError::AboveLimit => Self::TargetAboveLimit,
            PowError::HashAboveTarget => Self::HashAboveTarget,
        }
    }
}
