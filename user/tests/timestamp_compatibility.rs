//! Integration tests to verify the migration from time crate to chrono doesn't break anything
//!
//! TODO: Remove this once the time crate is removed

use serde::{Deserialize, Serialize};
use serde_json;
use slippi_user::direct_codes::{DirectCode, DirectCodes};
use time::OffsetDateTime;

/// Old time crate implementation for comparison (will be removed after
/// we're confident in the migration)
mod old_time_impl {
    use serde::{Deserialize, Serialize};
    use time::macros::format_description;
    use time::{Date, OffsetDateTime, Time};

    pub fn serialize<S>(datetime: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        datetime.unix_timestamp().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;

        if let Some(timestamp) = value.as_i64() {
            return OffsetDateTime::from_unix_timestamp(timestamp).map_err(serde::de::Error::custom);
        }

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
}

/// Test struct using the old time crate implementation
#[derive(Serialize, Deserialize, Debug, Clone)]
struct TestStructOld {
    #[serde(with = "old_time_impl")]
    last_played: OffsetDateTime,
}

/// Test struct using the new DirectCode implementation (chrono)
type TestStructNew = DirectCode;

#[test]
fn test_unix_timestamp_parsing_compatibility() {
    // Test various unix timestamps
    let test_cases = vec![
        1640995200i64, // 2022-01-01 00:00:00 UTC
        1609459200i64, // 2021-01-01 00:00:00 UTC
        1577836800i64, // 2020-01-01 00:00:00 UTC
        0i64,          // Unix epoch
        2147483647i64, // Max 32-bit signed int
    ];

    for timestamp in test_cases {
        let json_value = serde_json::json!({"last_played": timestamp});
        let json_value_with_code = serde_json::json!({
            "lastPlayed": timestamp,
            "connectCode": "TEST#123"
        });

        // Parse with old implementation
        let old_result: Result<TestStructOld, _> = serde_json::from_value(json_value);

        // Parse with new implementation (DirectCode)
        let new_result: Result<TestStructNew, _> = serde_json::from_value(json_value_with_code);

        assert!(old_result.is_ok(), "Old implementation failed for timestamp {}", timestamp);
        assert!(new_result.is_ok(), "New implementation failed for timestamp {}", timestamp);

        let old_dt = old_result.unwrap().last_played;
        let new_dt = new_result.unwrap().last_played;

        // Compare unix timestamps
        assert_eq!(
            old_dt.unix_timestamp(),
            new_dt.timestamp(),
            "Unix timestamps don't match for input {}",
            timestamp
        );
    }
}

#[test]
fn test_direct_codes_add_or_update_functionality() {
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    // Create a dummy path for testing (we won't actually write to it in this test)
    let temp_path = PathBuf::from("");

    let direct_codes = DirectCodes::load(temp_path);

    // Test adding a new code
    let test_code = "TEST#123".to_string();
    direct_codes.add_or_update_code(test_code.clone());

    // Verify the code was added
    assert_eq!(direct_codes.len(), 1);
    assert_eq!(direct_codes.get(0), "TEST#123");

    // Add a second code to test ordering
    thread::sleep(Duration::from_millis(5)); // Small delay to ensure different timestamps
    let test_code2 = "TEST#456".to_string();
    direct_codes.add_or_update_code(test_code2.clone());

    assert_eq!(direct_codes.len(), 2);

    // Most recent should be first
    assert_eq!(direct_codes.get(0), "TEST#456");
    assert_eq!(direct_codes.get(1), "TEST#123");

    // Test updating the first code (TEST#123), should move to front
    thread::sleep(Duration::from_millis(10));
    direct_codes.add_or_update_code(test_code.clone());

    assert_eq!(direct_codes.len(), 2); // Still 2 codes
    // Order should've been updated again
    assert_eq!(direct_codes.get(0), "TEST#123");
    assert_eq!(direct_codes.get(1), "TEST#456");
}

#[test]
fn test_direct_codes_last_played_serialization() {
    let test_direct_code = DirectCode {
        connect_code: "TEST#123".to_string(),
        last_played: chrono::Utc::now(),
    };

    // Serialize to JSON and verify it contains a unix timestamp
    let serialized = serde_json::to_value(&test_direct_code).unwrap();

    // The lastPlayed field should be a number (unix timestamp)
    assert!(
        serialized["lastPlayed"].is_number(),
        "lastPlayed should serialize to a number (unix timestamp), got: {:?}",
        serialized["lastPlayed"]
    );

    // And it should be a positive integer representing a recent timestamp
    let timestamp = serialized["lastPlayed"].as_i64().unwrap();
    let current_time = chrono::Utc::now().timestamp();

    // Should be within a reasonable range (last 10 years to 10 seconds from now)
    let ten_years_ago = current_time - (10 * 365 * 24 * 60 * 60);
    let ten_seconds_future = current_time + 10;

    assert!(
        timestamp >= ten_years_ago && timestamp <= ten_seconds_future,
        "Unix timestamp {} is not in reasonable range [{}, {}]",
        timestamp,
        ten_years_ago,
        ten_seconds_future
    );
}

#[test]
fn test_legacy_timestamp_deserialization() {
    // Test various legacy YYYYMMDDTHHMMSS timestamp formats
    let test_cases = vec![
        ("20220101T000000", 1640995200i64), // 2022-01-01 00:00:00 UTC
        ("20211231T235959", 1640995199i64), // 2021-12-31 23:59:59 UTC
        ("20200229T120000", 1582977600i64), // 2020-02-29 12:00:00 UTC (leap year)
        ("20191225T183045", 1577298645i64), // 2019-12-25 18:30:45 UTC
        ("19700101T000001", 1i64),          // 1970-01-01 00:00:01 UTC (near epoch)
    ];

    for (legacy_str, expected_timestamp) in test_cases {
        // Test with old time crate implementation
        let old_json = serde_json::json!({"last_played": legacy_str});
        let old_result: Result<TestStructOld, _> = serde_json::from_value(old_json);

        // Test with new chrono implementation (chrono)
        let new_json = serde_json::json!({
            "lastPlayed": legacy_str,
            "connectCode": "TEST#123"
        });
        let new_result: Result<TestStructNew, _> = serde_json::from_value(new_json);

        println!("Testing legacy timestamp: {} -> expected: {}", legacy_str, expected_timestamp);

        if let Err(ref e) = old_result {
            println!("Old implementation error: {:?}", e);
        }
        if let Err(ref e) = new_result {
            println!("New implementation error: {:?}", e);
        }

        assert!(
            old_result.is_ok(),
            "Old implementation failed for legacy timestamp {}",
            legacy_str
        );
        assert!(
            new_result.is_ok(),
            "New implementation failed for legacy timestamp {}",
            legacy_str
        );

        let old_dt = old_result.unwrap().last_played;
        let new_dt = new_result.unwrap().last_played;

        // Compare unix timestamps
        assert_eq!(
            old_dt.unix_timestamp(),
            new_dt.timestamp(),
            "Unix timestamps don't match for legacy string {}",
            legacy_str
        );

        // Verify the timestamp matches expected value
        assert_eq!(
            new_dt.timestamp(),
            expected_timestamp,
            "Parsed timestamp doesn't match expected for {}",
            legacy_str
        );
    }
}
