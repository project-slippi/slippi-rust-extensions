//! Direct codes are used on the connect screen as a form of history (codes
//! that have been recently connected to).

use std::borrow::Cow;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

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
    pub last_played: DateTime<Utc>,
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
                    tracing::error!(target: Log::SlippiOnline, ?error, "Unable to parse direct codes file");
                },
            },

            Err(error) => {
                tracing::error!(target: Log::SlippiOnline, ?error, "Unable to read direct codes file");
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

        // Create UTC timestamp (equivalent to the old OffsetDateTime::now_utc())
        let last_played = Utc::now();

        let mut codes = self.codes.lock().expect("Unable to lock codes for autocomplete");

        let mut found = false;
        for entry in codes.iter_mut() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json;

    #[test]
    fn test_legacy_timestamp_deserialization() {
        use serde_json::json;

        // This should represent Jan 1, 2022, 00:00:00 UTC
        let legacy_timestamp_str = "20220101T000000";
        let expected_timestamp = 1640995200i64;

        // mocked JSON that matches the DirectCode struct’s fields
        let json_data = json!({
            "connectCode": "TEST#LEGACY",
            "lastPlayed": legacy_timestamp_str
        });

        // Deserialize into DirectCode
        let deserialized: DirectCode = serde_json::from_value(json_data).unwrap();

        // Check that the timestamp matches
        assert_eq!(deserialized.connect_code, "TEST#LEGACY");
        assert_eq!(deserialized.last_played.timestamp(), expected_timestamp);
    }

    #[test]
    fn test_direct_code_serialization_roundtrip() {
        let known_timestamp = 1640995200i64; // 2022-01-01 00:00:00 UTC
        let known_datetime = Utc.timestamp_opt(known_timestamp, 0).unwrap();

        let direct_code = DirectCode {
            connect_code: "TEST#KNOWN".to_string(),
            last_played: known_datetime,
        };

        // Serialize to JSON
        let json = serde_json::to_value(&direct_code).unwrap();

        assert_eq!(json["connectCode"], "TEST#KNOWN");
        assert_eq!(json["lastPlayed"], known_timestamp);

        // Verify we can deserialize it
        let deserialized: DirectCode = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.connect_code, "TEST#KNOWN");
        assert_eq!(deserialized.last_played.timestamp(), known_timestamp);
    }

    #[test]
    fn test_timestamp_ordering_behavior() {
        use std::path::PathBuf;
        use std::thread;
        use std::time::Duration;

        let dummy_path = PathBuf::from("");
        let direct_codes = DirectCodes::load(dummy_path);

        // Add codes with slight delays to ensure different timestamps
        direct_codes.add_or_update_code("FRST#001".to_string());
        thread::sleep(Duration::from_millis(5));
        direct_codes.add_or_update_code("SCND#002".to_string());
        thread::sleep(Duration::from_millis(5));

        // Most recently added or updated should be first
        assert_eq!(direct_codes.get(0), "SCND#002");
        assert_eq!(direct_codes.get(1), "FRST#001");

        // Update the first code, should move it to front
        direct_codes.add_or_update_code("FRST#001".to_string());

        // Now FIRST should be first
        assert_eq!(direct_codes.get(0), "FRST#001");
        assert_eq!(direct_codes.get(1), "SCND#002");
    }
}
