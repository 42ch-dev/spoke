//! Generated scalar pins: every `DateTime<Utc>` newtype parses through the one
//! strict RFC 3339 grammar (spoke-data-model §Shared wire scalars), and
//! `StoredTimestamp` keeps the stored spelling byte-for-byte.

use std::str::FromStr;

use serde_json::json;
use spoke_schemas::data::revision_snapshot::StoredTimestamp;
use spoke_schemas::Timestamp;

#[test]
fn generated_timestamp_refuses_relaxed_spellings_on_both_entry_points() {
    for value in [
        "2026-11-23 10:00:00Z",
        "2026-11-23 10:00:00",
        "2026-11-23T10:00:00 UTC",
        "2026-11-23T10:00:00utc",
        "\u{a0}2026-11-23T10:00:00Z",
        "2026-11-23T10:00:00Z\n",
        "2026-11-23T10:00:5Z",
        "2026-11-23T10:00:00",
    ] {
        assert!(
            serde_json::from_value::<Timestamp>(json!(value)).is_err(),
            "serde accepted {value:?}"
        );
        assert!(
            Timestamp::from_str(value).is_err(),
            "FromStr accepted {value:?}"
        );
    }
}

#[test]
fn accepted_timestamps_encode_byte_identically() {
    for (input, encoded) in [
        ("2026-07-23T09:00:00Z", "2026-07-23T09:00:00Z"),
        ("2026-07-23t09:00:00z", "2026-07-23T09:00:00Z"),
        ("2026-07-23T09:00:00.5+08:00", "2026-07-23T01:00:00.500Z"),
        ("2026-07-23T09:00:00.1234567890Z", "2026-07-23T09:00:00.123456789Z"),
        ("2026-07-23T09:00:00-00:00", "2026-07-23T09:00:00Z"),
    ] {
        let timestamp: Timestamp =
            serde_json::from_value(json!(input)).expect("grammar-admitted input");
        assert_eq!(
            serde_json::to_value(&timestamp).expect("serializes"),
            json!(encoded),
            "encode differs for {input:?}"
        );
    }
}

#[test]
fn stored_timestamp_replays_the_stored_bytes() {
    for value in [
        "2026-07-23T09:00:00.5+08:00",
        "2026-07-23t09:00:00z",
        "2026-07-23T09:00:00.1234567890Z",
    ] {
        let stored: StoredTimestamp = serde_json::from_value(json!(value)).expect("admits grammar");
        assert_eq!(serde_json::to_value(&stored).expect("serializes"), json!(value));
    }
    for value in ["2026-11-23 10:00:00Z", "2026-11-23T10:00:00 UTC"] {
        assert!(serde_json::from_value::<StoredTimestamp>(json!(value)).is_err());
    }
}
