//! Strict RFC 3339 `date-time` parsing shared by every generated `Timestamp`
//! newtype and `StoredTimestamp` (spoke-data-model §Shared wire scalars).
//!
//! The ASCII grammar gate supplies the accepted spelling; `chrono` supplies
//! calendar validity, leap seconds and the offset ceiling. `chrono` alone is
//! too permissive: `DateTime::parse_from_rfc3339` admits a space in place of
//! `T` (`chrono`'s parser accepts `full-date " " full-time` for readability).

use chrono::{DateTime, ParseError, Utc};
use serde::de::{Deserializer, Error as DeError, Visitor};
use std::fmt;

/// Parse one RFC 3339 `date-time` with an explicit offset (`Z` or `±hh:mm`).
pub fn parse_strict(value: &str) -> Result<DateTime<Utc>, ParseError> {
    if !grammar_ok(value.as_bytes()) {
        // shortcut: reuse chrono's error type so every caller keeps its
        // `ParseError` signature; the failure message is not part of the
        // contract. Upgrade if a distinguishable grammar error becomes
        // observable.
        return Err(DateTime::parse_from_rfc3339("").unwrap_err());
    }
    DateTime::parse_from_rfc3339(value).map(|parsed| parsed.with_timezone(&Utc))
}

struct StrictVisitor;

impl Visitor<'_> for StrictVisitor {
    type Value = DateTime<Utc>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an RFC 3339 date-time with an explicit offset")
    }

    fn visit_str<E: DeError>(self, value: &str) -> Result<Self::Value, E> {
        parse_strict(value).map_err(E::custom)
    }
}

struct OptionalStrictVisitor;

impl<'de> Visitor<'de> for OptionalStrictVisitor {
    type Value = Option<DateTime<Utc>>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an RFC 3339 date-time with an explicit offset, or null")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_strict(deserializer).map(Some)
    }
}

/// Strict decode for a generated direct `DateTime<Utc>` wire field.
pub fn deserialize_strict<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_str(StrictVisitor)
}

/// Strict decode for a generated `Option<DateTime<Utc>>` wire field.
pub fn deserialize_optional_strict<'de, D>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_option(OptionalStrictVisitor)
}

fn grammar_ok(bytes: &[u8]) -> bool {
    let mut i = 0;
    if !digits(bytes, &mut i, 4) || !lit(bytes, &mut i, b'-') {
        return false;
    }
    if !digits(bytes, &mut i, 2) || !lit(bytes, &mut i, b'-') {
        return false;
    }
    if !digits(bytes, &mut i, 2) || !matches!(bytes.get(i), Some(b'T') | Some(b't')) {
        return false;
    }
    i += 1;
    if !digits(bytes, &mut i, 2) || !lit(bytes, &mut i, b':') {
        return false;
    }
    if !digits(bytes, &mut i, 2) || !lit(bytes, &mut i, b':') {
        return false;
    }
    if !digits(bytes, &mut i, 2) {
        return false;
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if !offset(bytes, &mut i) {
        return false;
    }
    i == bytes.len()
}

fn digits(bytes: &[u8], i: &mut usize, count: usize) -> bool {
    for _ in 0..count {
        if !bytes.get(*i).is_some_and(u8::is_ascii_digit) {
            return false;
        }
        *i += 1;
    }
    true
}

fn lit(bytes: &[u8], i: &mut usize, byte: u8) -> bool {
    if bytes.get(*i) != Some(&byte) {
        return false;
    }
    *i += 1;
    true
}

fn offset(bytes: &[u8], i: &mut usize) -> bool {
    match bytes.get(*i) {
        Some(&byte) if byte == b'Z' || byte == b'z' => {
            *i += 1;
            true
        }
        Some(&byte) if byte == b'+' || byte == b'-' => {
            *i += 1;
            digits(bytes, i, 2) && lit(bytes, i, b':') && digits(bytes, i, 2)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_strict;

    #[test]
    fn accepts_the_wire_grammar() {
        for value in [
            "2026-07-23T09:00:00Z",
            "2026-07-23t09:00:00z",
            "2026-07-23T09:00:00.1234567890+08:00",
            "2026-07-23T09:00:00-00:00",
            "2026-07-23T09:00:60Z",
            "0000-01-01T00:00:00Z",
            "2026-07-23T09:00:00+23:59",
        ] {
            assert!(parse_strict(value).is_ok(), "expected {value} to parse");
        }
    }

    #[test]
    fn refuses_spellings_chrono_alone_admits() {
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
            assert!(parse_strict(value).is_err(), "expected {value} to refuse");
        }
    }

    #[test]
    fn refuses_values_outside_the_calendar_or_offset_range() {
        for value in [
            "2026-02-30T09:00:00Z",
            "2026-13-01T09:00:00Z",
            "2026-07-23T24:00:00Z",
            "2026-07-23T09:00:00+24:00",
            "2026-07-23T09:00:00+23:60",
            "2026-07-23T09:00:00+08",
        ] {
            assert!(parse_strict(value).is_err(), "expected {value} to refuse");
        }
    }
}
