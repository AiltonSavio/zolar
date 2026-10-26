//! Stable numbered errors, mirrored by Codama.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum VendingError {
    WrongAccount = 1,
    BadConfig = 2,
    SoldOut = 3,
    InvalidProof = 4,
    BadPublicValues = 5,
    WrongMachine = 6,
    WrongDiversifier = 7,
    WrongViewingKey = 8,
    WrongBranch = 9,
    UnknownRoot = 10,
    RelayHalted = 11,
    RelayNotReadable = 12,
    Underpaid = 13,
    PaymentExpired = 14,
    TagUsed = 15,
    MissingReservation = 16,
    ReleaseTooEarly = 17,
    ArithmeticOverflow = 18,
    BadTokenAccount = 19,
    AlreadyInitialized = 20,
}
