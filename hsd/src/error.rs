use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("archive is truncated: needed {needed} bytes, have {have}")]
    Truncated { needed: usize, have: usize },

    #[error("header says the file is {header} bytes but it is {actual}")]
    SizeMismatch { header: u32, actual: usize },

    #[error("pointer at data offset 0x{field:x} targets 0x{target:x}, outside the data section")]
    PointerOutOfRange { field: u32, target: u32 },

    #[error("root {name} targets 0x{target:x}, outside the data section")]
    RootOutOfRange { name: String, target: u32 },

    #[error("field at data offset 0x{field:x} straddles a struct boundary")]
    FieldStraddlesBoundary { field: u32 },

    #[error("external reference {name} has a malformed chain at data offset 0x{at:x}")]
    BadChain { name: String, at: u32 },

    #[error("external reference {name} has no locations")]
    EmptyRef { name: String },

    #[error("malformed link symbol {symbol}: {reason}")]
    BadLocator { symbol: String, reason: String },

    #[error("could not read disc file {file}: {reason}")]
    Source { file: String, reason: String },

    #[error("disc file {file} has no struct starting at file offset 0x{offset:x}")]
    NotAStruct { file: String, offset: u32 },

    #[error("link {symbol} does not resolve in the disc file: {reason}")]
    BadPath { symbol: String, reason: String },

    #[error("linked asset {symbol} does not match its recorded hash (expected {expected}, found {found})")]
    HashMismatch {
        symbol: String,
        expected: String,
        found: String,
    },

    #[error("extent {extent} is a root and cannot be linked")]
    CannotLinkRoot { extent: usize },

    #[error("extent {extent} is not referenced by any pointer")]
    Unreferenced { extent: usize },

    #[error("external reference {name} has a location inside the subtree being linked")]
    RefInsideLinkedSubtree { name: String },

    #[error("extent index {extent} is out of range")]
    NoSuchExtent { extent: usize },
}
