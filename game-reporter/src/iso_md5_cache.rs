//! On-disk cache for ISO MD5 hash results, keyed by ISO path and invalidated
//! by file size + mtime. Hashing a multi-gigabyte ISO from a slow drive can
//! take a noticeable amount of time, so we persist results between runs.
//!
//! The file is stored as gzipped JSON. The compression isn't for size — it's
//! to make casual editing less inviting.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};

use dolphin_integrations::Log;

pub(crate) const CACHE_FILE_NAME: &str = "iso-cache";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    mtime_secs: i64,
    size: u64,
    hash: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    entries: HashMap<String, CacheEntry>,
}

/// Returns a cached hash for `iso_path` if one exists and the file's size and
/// mtime still match what was recorded.
pub(crate) fn lookup(cache_path: &Path, iso_path: &str) -> Option<String> {
    let (mtime_secs, size) = stat(iso_path)?;
    let cache = load(cache_path)?;
    let entry = cache.entries.get(iso_path)?;

    if entry.mtime_secs == mtime_secs && entry.size == size {
        Some(entry.hash.clone())
    } else {
        None
    }
}

/// Records `hash` for `iso_path` along with the file's current size and mtime.
/// Failures are logged but otherwise ignored - the cache is an optimization,
/// not a source of truth.
pub(crate) fn store(cache_path: &Path, iso_path: &str, hash: &str) {
    let Some((mtime_secs, size)) = stat(iso_path) else {
        return;
    };

    let mut cache = load(cache_path).unwrap_or_default();
    cache.entries.insert(
        iso_path.to_string(),
        CacheEntry {
            mtime_secs,
            size,
            hash: hash.to_string(),
        },
    );

    if let Err(error) = save(cache_path, &cache) {
        tracing::warn!(target: Log::SlippiOnline, ?error, "Failed to write ISO MD5 cache");
    }
}

fn stat(iso_path: &str) -> Option<(i64, u64)> {
    let metadata = fs::metadata(iso_path).ok()?;
    let size = metadata.len();
    let mtime = metadata.modified().ok()?;
    let mtime_secs = match mtime.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    };
    Some((mtime_secs, size))
}

fn load(cache_path: &Path) -> Option<Cache> {
    let file = File::open(cache_path).ok()?;
    let mut decoder = GzDecoder::new(file);
    let mut json = String::new();
    decoder.read_to_string(&mut json).ok()?;
    serde_json::from_str(&json).ok()
}

fn save(cache_path: &Path, cache: &Cache) -> std::io::Result<()> {
    let json = serde_json::to_vec(cache)?;
    let file = File::create(cache_path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(&json)?;
    encoder.finish()?;
    Ok(())
}

/// Convenience helper for building the canonical cache path inside the user
/// config folder.
pub(crate) fn path_in(user_config_folder: &Path) -> PathBuf {
    user_config_folder.join(CACHE_FILE_NAME)
}
