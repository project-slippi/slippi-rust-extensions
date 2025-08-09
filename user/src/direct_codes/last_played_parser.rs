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
use time::{Date, OffsetDateTime, Time};

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

    // This splits old timestamps (e.g: "20230323T181928") and parses date and time separately
    // then combines them back into an OffsetDateTime. It is an unfortunate workaround to the
    // time crate using some completely custom format that attempts to be better than the strftime
    // utilities that everything else uses, along with having woeful documentation on how to parse
    // out custom datetime strings.
    //
    // (Using a format of "[year][month][day]T[hour][minute][second]" produces
    // an error informing that there's insufficient information to parse, and there's nothing
    // further to debug past there. This code is something that will get ripped out in the future
    // anyway after enough time for people to be migrated to the unix timestamp format.)
    //
    // (Read: I should have just used chrono/jiff. I don't have bandwidth to migrate things atm.)
    if let Some(datetime_str) = value.as_str() {
        let split: Vec<&str> = datetime_str.split("T").collect();

        if split.len() == 2 {
            let date_fmt = format_description!("[year][month][day]");
            let date = Date::parse(&split[0], &date_fmt).map_err(serde::de::Error::custom)?;

            let time_fmt = format_description!("[hour][minute][second]");
            let time = Time::parse(&split[1], &time_fmt).map_err(serde::de::Error::custom)?;

            return Ok(OffsetDateTime::new_utc(date, time));
        }
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
