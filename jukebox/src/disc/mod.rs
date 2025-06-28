use std::fs::File;
use std::io::{Read, Seek};

use crate::JukeboxError::*;
use crate::Result;

mod ciso;

#[derive(Debug, Clone, Copy)]
pub(crate) enum IsoKind {
    Standard,
    Ciso,
    Unknown,
}

/// Given an iso file, determine what kind it is
pub(crate) fn get_iso_kind(iso: &mut File) -> Result<IsoKind> {
    // Get the first four bytes
    iso.rewind().map_err(IsoSeek)?;
    let mut initial_bytes = [0; 4];
    iso.read_exact(&mut initial_bytes).map_err(IsoRead)?;

    // Get the four bytes at 0x1c
    iso.seek(std::io::SeekFrom::Start(0x1c)).map_err(IsoSeek)?;
    let mut dvd_magic_bytes = [0; 4];
    iso.read_exact(&mut dvd_magic_bytes).map_err(IsoRead)?;

    match (initial_bytes, dvd_magic_bytes) {
        // DVD Magic Word
        (_, [0xc2, 0x33, 0x9F, 0x3D]) => Ok(IsoKind::Standard),
        // CISO header
        ([0x43, 0x49, 0x53, 0x4F], _) => Ok(IsoKind::Ciso),
        _ => Ok(IsoKind::Unknown),
    }
}

/// When we want to read data from any given iso file, but we only know the
/// offset for a standard disc image, we need a way to be able to get the
/// _actual_ offset for the file we have on hand. This can vary depending on the
/// kind of disc image that we are dealing with (standard vs ciso, for example).
///
/// This HoF returns a fn that can be used to locate the true offset. If the
/// returned fn returns `None`, then the desired offset maps to nothing in the
/// provided ISO.
///
/// Example Usage:
/// ```ignore
/// let iso = File::open("/foo/bar.iso")?;
/// let get_true_offset = create_offset_locator_fn(&mut iso)?;
/// let offset = get_true_offset(0x424);
/// ```
pub(crate) fn create_offset_locator_fn(iso: &mut File) -> Result<impl Fn(u64) -> Option<u64> + use<>> {
    // Get the ciso header (block size and block map) of the provided file.
    // If the file is not a ciso, this will be `None`
    let ciso_header = ciso::get_ciso_header(iso)?;

    Ok(move |offset| match ciso_header {
        Some(ciso_header) => ciso::get_ciso_offset(&ciso_header, offset),
        None => Some(offset),
    })
}
