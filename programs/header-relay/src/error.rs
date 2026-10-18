use zcash_header::{
    difficulty::{DifficultyError, TimeError},
    equihash,
    pow::PowError,
};

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
    /// The header is more than `REORG_WINDOW` blocks below the tip.
    TooOld = 23,
    /// The Store is below `MIN_CAPACITY`; call `GrowStore` first.
    StoreTooSmall = 24,
    /// `nVersion` is below 4.
    InvalidVersion = 25,
    WorkOverflow = 26,
    /// `nBits` is not the target the difficulty adjustment gives for this block.
    BadDifficulty = 27,
    /// `nTime` is not after the median-time-past of the previous eleven blocks.
    TimeNotAfterMedian = 28,
    /// `nTime` is more than 90 minutes after the median-time-past.
    TimeTooFarAhead = 29,
    /// The parent's chain is no longer in the Store far enough back to compute the difficulty.
    ContextUnavailable = 30,
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
            PowError::WorkOverflow => Self::WorkOverflow,
        }
    }
}

impl From<TimeError> for RelayError {
    fn from(e: TimeError) -> Self {
        match e {
            TimeError::NotAfterMedian => Self::TimeNotAfterMedian,
            TimeError::TooFarAhead => Self::TimeTooFarAhead,
            TimeError::ContextTooShort => Self::ContextUnavailable,
        }
    }
}

impl From<DifficultyError> for RelayError {
    fn from(e: DifficultyError) -> Self {
        match e {
            DifficultyError::ContextTooShort => Self::ContextUnavailable,
            DifficultyError::InvalidContextBits => Self::InvalidCheckpoint,
        }
    }
}
