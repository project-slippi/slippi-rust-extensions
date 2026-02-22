//! Implements deserialization/parsing the `last_played` field from direct
//! code file payloads. This will decode from either a unix timestamp *or*
//! an older used datetime string format.
//!
//! Subsequent writes to the direct codes file(s) will have their timstamps
//! written as i64 unix timestamps. This could potentially be done away with
//! after a few releases - just stub in the time crate macro for auto-generating
//! unix timestamp handling code.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Serializes a timestamp as a unix timestamp (`i64`).
pub fn serialize<S>(datetime: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    datetime.timestamp().serialize(serializer)
}

/// Takes two string slices, like "YYYYMMDD" and "HHMMSS" and combines them (using chrono)
/// into a UTC datetime.
fn parse_to_utc(date_part: &str, time_part: &str) -> Result<DateTime<Utc>, String> {
    // Parse the date part (YYYYMMDD)
    let naive_date = NaiveDate::parse_from_str(date_part, "%Y%m%d").map_err(|e| format!("Invalid date: {}", e))?;

    // Parse the time part (HHMMSS)
    let naive_time = NaiveTime::parse_from_str(time_part, "%H%M%S").map_err(|e| format!("Invalid time: {}", e))?;

    // Combine into a naive datetime
    let naive_datetime = naive_date.and_time(naive_time);

    // Interpret as UTC
    Ok(Utc.from_utc_datetime(&naive_datetime))
}

/// Attempts deserialiazation of the `last_played` field, by first checking if it's a
/// unix timestamp and falling back to the legacy timestamp format if not.
pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;

    // Parse unix timestamp
    if let Some(timestamp) = value.as_i64() {
        // Convert i64 to UTC datetime
        let datetime: DateTime<Utc> =
            DateTime::from_timestamp(timestamp, 0).ok_or_else(|| serde::de::Error::custom("Invalid UTC timestamp"))?;

        return Ok(datetime);
    }

    // Parsing the legacy datetime string format
    if let Some(datetime_str) = value.as_str() {
        // Original time crate format: YYYYMMDDTHHMMSS
        // e.g., "20211231T050000" = Dec 31, 2021, 5:00 AM UTC
        // or "20230323T181928" = Mar 23, 2023, 6:19 PM UTC

        let parts: Vec<&str> = datetime_str.split('T').collect();
        if parts.len() == 2 {
            let date_part = parts[0]; // should be a string like YYYYMMDD
            let time_part = parts[1]; // should be a string like HHMMSS

            if date_part.len() != 8 || time_part.len() != 6 {
                return Err(serde::de::Error::custom(format!(
                    "Invalid datetime string format: {}",
                    datetime_str
                )));
            }

            let dt = parse_to_utc(date_part, time_part).map_err(|e| {
                serde::de::Error::custom(format!("Failed to parse legacy timestamp string into unix timestamp: {}", e))
            })?;
            return Ok(dt);
        }

        return Err(serde::de::Error::custom(format!(
            "Invalid datetime string format: {}",
            datetime_str
        )));
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
    "[year][month][day]T[hour][minute][second]"
);*/
