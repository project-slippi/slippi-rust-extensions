//! Implements deserialization/parsing the `last_played` field from direct
//! code file payloads. This will decode from either a unix timestamp *or*
//! an older used datetime string format.
//!
//! Subsequent writes to the direct codes file(s) will have their timstamps
//! written as i64 unix timestamps. This could potentially be done away with
//! after a few releases - just stub in the time crate macro for auto-generating
//! unix timestamp handling code.

use serde::{Deserialize, Serialize};
use time::macros::format_description;
use time::OffsetDateTime;

/// Serializes a timestamp as a unix timestamp (`i64`).
pub fn serialize<S>(datetime: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    datetime.unix_timestamp().serialize(serializer)
}

/// Attempts deserialiazation of the `last_played` field, by first checking if it's a
/// unix timestamp and falling back to the older timestamp format if not.
pub fn deserialize<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;

    if let Some(timestamp) = value.as_i64() {
        return OffsetDateTime::from_unix_timestamp(timestamp).map_err(serde::de::Error::custom);
    }

    if let Some(datetime_str) = value.as_str() {
        let tsfmt = format_description!("[year][month][day]T[offset_hour][offset_minute][offset_second]");

        return OffsetDateTime::parse(datetime_str, &tsfmt).map_err(serde::de::Error::custom);
    }

    Err(serde::de::Error::custom(format!(
        "Invalid last_played type in direct codes file: {:?}",
        value
    )))
}

// Auto-generate serde parsers for the lastPlayed JSON field.
// Once we hit a point where we could just assume unix timestamps for all players, this module
// could go away and this macro could just be shoved into `mod.rs` - probably with a bit of
// tweaking but that's the gist of things.
/*time::serde::format_description!(
    last_played_parser,
    OffsetDateTime,
    "[year][month][day]T[offset_hour][offset_minute][offset_second]"
);*/
