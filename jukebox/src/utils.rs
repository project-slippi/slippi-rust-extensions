use std::fs::File;
use std::io::{Read, Seek};

use crate::{JukeboxError::*, Result};

/// Get a copy of the `size` bytes in `file` at `offset`
pub(crate) fn copy_bytes_from_file(file: &mut File, offset: u64, size: usize) -> Result<Vec<u8>> {
    file.seek(std::io::SeekFrom::Start(offset)).map_err(IsoSeek)?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(IsoRead)?;
    Ok(bytes)
}
