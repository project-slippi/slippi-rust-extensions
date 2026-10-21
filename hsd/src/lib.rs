//! HAL sysdolphin archive support for Slippi.
//!
//! An archive (`.dat` / `.usd`) is a header, a data section, a relocation table listing every
//! pointer field in the data section, a table of exported root symbols, a table of imported
//! external symbols, and a string table. Pointers are offsets into the data section.
//!
//! This crate models the data section as a sequence of *extents*: the byte ranges between
//! consecutive pointer targets. Every pointer and root points at the start of an extent, which
//! makes copying assets between archives a matter of copying extents and rebasing pointers, with
//! no knowledge of the structures inside.
//!
//! Slippi ships game files whose Nintendo-authored assets are replaced by external symbols of the
//! form `slp:<disc file>:0x<offset>`. [`resolve`] copies those assets in from the user's own disc
//! at load time, so the shipped file carries none of Nintendo's data.

pub mod archive;
pub mod autolink;
pub mod error;
pub mod hash;
pub mod link;

pub use archive::{Archive, Extent, ExternRef, Pointer, Root};
pub use autolink::{Options as AutolinkOptions, Report, SourceIndex, autolink};
pub use error::Error;
pub use link::{Anchor, LinkStatus, Locator, Resolution, ResolvedLink, difference, link, locate, resolve};

/// Prefix of external symbols this crate resolves. Other symbols are left untouched.
pub const LINK_PREFIX: &str = "slp:";

pub type Result<T> = std::result::Result<T, Error>;
