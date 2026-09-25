//! Resolves Slippi asset links in game files handed over by the C++ game file loader.
//!
//! A shipped game file may reference assets in the user's own disc through external symbols of
//! the form `slp:<disc file>:0x<offset>`. Dolphin's loader calls [`slprs_gamefile_resolve`] with
//! the file's bytes and a callback that reads disc files; the resolved archive comes back as a
//! buffer that must be released with [`slprs_gamefile_free`].

use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::slice;
use std::time::Instant;

use dolphin_integrations::Log;
use slippi_hsd::{Archive, Locator, resolve};

use crate::c_str_to_string;

/// The file was resolved. `data` holds the result and must be freed with `slprs_gamefile_free`.
pub const SLPRS_GAMEFILE_RESOLVED: i32 = 0;

/// The file has no links, or is not an archive. Use it as it was passed in. `data` is null.
pub const SLPRS_GAMEFILE_NO_LINKS: i32 = 1;

/// A link could not be resolved; the reason was logged. `data` is null.
pub const SLPRS_GAMEFILE_FAILED: i32 = 2;

/// Result of `slprs_gamefile_resolve`.
#[repr(C)]
pub struct SlippiResolvedGameFile {
    pub data: *mut u8,
    pub len: usize,
    pub status: i32,
}

/// Reads a file from the running disc. Given a file name, point `out_data` and `out_len` at the
/// file's bytes and return true, or return false if the file does not exist. The bytes only need
/// to stay valid until the callback is invoked again or `slprs_gamefile_resolve` returns; Rust
/// copies them.
pub type SlippiReadDiscFileFn = Option<unsafe extern "C" fn(ctx: *mut c_void, file_name: *const c_char, out_data: *mut *const u8, out_len: *mut usize) -> bool>;

fn no_links() -> SlippiResolvedGameFile {
    SlippiResolvedGameFile {
        data: ptr::null_mut(),
        len: 0,
        status: SLPRS_GAMEFILE_NO_LINKS,
    }
}

fn failed() -> SlippiResolvedGameFile {
    SlippiResolvedGameFile {
        data: ptr::null_mut(),
        len: 0,
        status: SLPRS_GAMEFILE_FAILED,
    }
}

/// Resolves the `slp:` links in the archive at `data` using `read_disc_fn` to fetch disc files.
/// `file_name` is only used for logging. `ctx` is passed through to the callback untouched.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_gamefile_resolve(
    file_name: *const c_char,
    data: *const u8,
    len: usize,
    ctx: *mut c_void,
    read_disc_fn: SlippiReadDiscFileFn,
) -> SlippiResolvedGameFile {
    let name = c_str_to_string(file_name, "slprs_gamefile_resolve", "file_name");
    if data.is_null() || len < slippi_hsd::archive::HEADER_SIZE as usize {
        return no_links();
    }

    // The C++ side owns this buffer for the duration of the call.
    let bytes = unsafe { slice::from_raw_parts(data, len) };

    // Cheap exit for archives without an external reference table.
    if u32::from_be_bytes([bytes[0x10], bytes[0x11], bytes[0x12], bytes[0x13]]) == 0 {
        return no_links();
    }

    let started = Instant::now();
    let archive = match Archive::parse(bytes) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(target: Log::SlippiOnline, "{name} is not a parseable archive, serving it as is: {e}");
            return no_links();
        },
    };
    let links: Vec<&str> = archive.refs.iter().map(|r| r.name.as_str()).filter(|n| Locator::is_link(n)).collect();
    if links.is_empty() {
        return no_links();
    }

    let mut disc_files: Vec<String> = Vec::new();
    let mut load = |disc_file: &str| -> Result<Vec<u8>, String> {
        let Some(read) = read_disc_fn else {
            return Err("no disc reader was provided".into());
        };
        let c_name = CString::new(disc_file).map_err(|_| "file name contains a NUL byte".to_string())?;
        let mut out_data: *const u8 = ptr::null();
        let mut out_len: usize = 0;
        // The callback is provided by our own C++ loader and follows the contract documented on
        // `SlippiReadDiscFileFn`.
        let ok = unsafe { read(ctx, c_name.as_ptr(), &mut out_data, &mut out_len) };
        if !ok || out_data.is_null() {
            return Err("not found on the disc".into());
        }
        disc_files.push(disc_file.to_string());
        Ok(unsafe { slice::from_raw_parts(out_data, out_len) }.to_vec())
    };

    let resolution = match resolve(&archive, &mut load) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(target: Log::SlippiOnline, "Could not resolve links in {name}: {e}");
            return failed();
        },
    };
    for warning in &resolution.warnings {
        tracing::warn!(target: Log::SlippiOnline, "{name}: {warning}");
    }
    let out = match resolution.archive.serialize() {
        Ok(bytes) => bytes.into_boxed_slice(),
        Err(e) => {
            tracing::error!(target: Log::SlippiOnline, "Could not serialize resolved {name}: {e}");
            return failed();
        },
    };

    tracing::info!(
        target: Log::SlippiOnline,
        "Resolved {} link(s) in {name} from {:?} in {:.1?}: {} bytes in, {} bytes out",
        links.len(),
        disc_files,
        started.elapsed(),
        len,
        out.len()
    );

    let len = out.len();
    SlippiResolvedGameFile {
        data: Box::into_raw(out) as *mut u8,
        len,
        status: SLPRS_GAMEFILE_RESOLVED,
    }
}

/// Releases a buffer returned by `slprs_gamefile_resolve`. Safe to call with a null `data`.
#[unsafe(no_mangle)]
pub extern "C" fn slprs_gamefile_free(result: SlippiResolvedGameFile) {
    if result.data.is_null() {
        return;
    }
    // Rebuild the boxed slice exactly as it was handed out.
    unsafe {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(result.data, result.len)));
    }
}
