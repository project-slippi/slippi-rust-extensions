//! Direct codes are used on the connect screen as a form of history (codes
//! that have been recently connected to).

use std::borrow::Cow;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use time::OffsetDateTime;

use dolphin_integrations::Log;

mod last_played_parser;

/// Indicates how a sort of the direct codes should be done.
#[derive(Debug)]
enum SortBy {
    // This sort type is not used at the moment, but was stubbed
    // out in the C++ version. It's kept around commented out for
    // marking potential future intentions.
    // Name,
    LastPlayed,
}

/// The actual payload that's serialized back and forth to disk.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct DirectCode {
    #[serde(rename = "connectCode", alias = "connect_code")]
    pub connect_code: String,

    #[serde(rename = "lastPlayed", alias = "last_played", with = "last_played_parser")]
    pub last_played: OffsetDateTime,
    // This doesn't exist yet and is stubbed to match the C++ version,
    // which had some inkling of it - and could always be used in the
    // future.
    // #[serde(rename = "favorite")]
    // pub is_favorite: Option<bool>
}

/// A wrapper around a list of direct codes. The main entry point for querying,
/// sorting, and adding codes. This type is thread safe and be freely cloned and
/// passed around, though realistically only the user manager should need it.
#[derive(Clone, Debug)]
pub struct DirectCodes {
    path: Arc<PathBuf>,
    codes: Arc<Mutex<Vec<DirectCode>>>,
}

impl DirectCodes {
    /// Given a `path` that points to a user direct codes JSON file, will attempt
    /// to load and deserialize the data. If either fails, this will log a message
    /// indicating there's an issue but it will not error out - the underlying payload
    /// will simply be empty.
    pub fn load(path: PathBuf) -> Self {
        tracing::info!(target: Log::SlippiOnline, ?path, "Attempting to load direct codes");

        let mut codes = Vec::new();

        match fs::read_to_string(path.as_path()) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(parsed) => {
                    codes = parsed;
                },

                Err(error) => {
                    tracing::error!(?error, "Unable to parse direct codes file");
                },
            },

            Err(error) => {
                tracing::error!(?error, "Unable to read direct codes file");
            },
        }

        Self {
            path: Arc::new(path),
            codes: Arc::new(Mutex::new(codes)),
        }
    }

    /// Sorts the underlying direct codes list by the `sort_by` parameter.
    fn sort(codes: &mut Vec<DirectCode>, sort_by: SortBy) {
        match sort_by {
            SortBy::LastPlayed => {
                codes.sort_by(|a, b| b.last_played.cmp(&a.last_played));
            },
        }
    }

    /// Returns the length of the underlying direct codes list.
    ///
    /// This could generally be done with `Deref`, but needing a custom `sort` leads
    /// me to think that this will be more clear in the long term how delegation is
    /// happening.
    pub fn len(&self) -> usize {
        let codes = self.codes.lock().expect("Unable to lock codes for len check");

        codes.len()
    }

    /// Attempts to get the connect code at the specified index.
    ///
    /// This utilizes `Cow` (Copy-On-Write) to avoid extra allocations where
    /// we don't perhaps need them.
    pub fn get(&self, index: usize) -> Cow<'static, str> {
        let mut codes = self.codes.lock().expect("Unable to lock codes for autocomplete");

        Self::sort(&mut codes, SortBy::LastPlayed);

        if let Some(entry) = codes.get(index) {
            return Cow::Owned(entry.connect_code.clone());
        }

        tracing::info!(target: Log::SlippiOnline, ?index, "Potential out of bounds name entry index");

        Cow::Borrowed(match index >= codes.len() {
            true => "1",
            false => "",
        })
    }

    /// Adds or updates a direct code.
    ///
    /// If it's an update, we're just updating the timestamp so that future sorts
    /// order it appropriately.
    pub fn add_or_update_code(&self, code: String) {
        tracing::warn!(target: Log::SlippiOnline, ?code, "Attempting to add or update direct code");

        let last_played = OffsetDateTime::now_utc();

        let mut codes = self.codes.lock().expect("Unable to lock codes for autocomplete");

        let mut found = false;
        for mut entry in codes.iter_mut() {
            if entry.connect_code == code {
                found = true;
                entry.last_played = last_played;
            }
        }

        if !found {
            codes.push(DirectCode {
                connect_code: code,
                last_played,
            });
        }

        // Consider moving this to a background thread if the performance of
        // `write_file` ever becomes an issue. In practice, it's never been one.
        Self::write_file(self.path.as_path(), &codes);
    }

    /* The below code is not used at the moment, but stubbed out to match the C++ side.
    /// Attempts to autocomplete a code based off of the start text.
    pub fn autocomplete(&self, start_text: &str) -> Option<String> {
        let mut codes = self.codes.lock()
            .expect("Unable to lock codes for autocomplete");

        Self::sort(&mut codes, SortBy::Time);

        for code in codes.iter() {
            if code.connect_code.as_str().starts_with(start_text) {
                return Some(code.connect_code.clone());
            }
        }

        None
    }*/

    /// Serializes and writes the contents of `codes` to disk at `path`.
    fn write_file(path: &Path, codes: &[DirectCode]) {
        match fs::File::create(path) {
            Ok(file) => {
                let mut writer = BufWriter::new(file);

                if let Err(error) = serde_json::to_writer(&mut writer, codes) {
                    tracing::error!(target: Log::SlippiOnline, ?error, "Unable to write direct codes to disk");
                    return;
                }

                if let Err(error) = writer.flush() {
                    tracing::error!(target: Log::SlippiOnline, ?error, "Unable to flush direct codes file to disk");
                    return;
                }
            },

            Err(error) => {
                tracing::error!(
                    target: Log::SlippiOnline,
                    ?error,
                    ?path,
                    "Unable to open direct codes file for write"
                );
            },
        }
    }
}
