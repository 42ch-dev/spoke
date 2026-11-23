//! Bounded wire integers (spoke-data-model §Shared wire scalars).
//!
//! `SchemaVersion` and the revision family keep their published Rust types
//! (`NonZeroU64` / `u64`); these helpers enforce the schema ceiling on the
//! serde, `FromStr` and `TryFrom` entry points. The inclusive revision bound
//! `[0, 2^53]` is asserted on the parsed integer value: an integral float
//! spelling (`9007199254740992.0`) is the endpoint, while an exact integer
//! token above it (`9007199254740993`) is refused without an `f64` round-trip.

use serde::de::{Deserializer, Error as DeError, Visitor};
use std::fmt;
use std::num::{NonZeroU64, ParseIntError};

/// `2^53 - 1`, the `SchemaVersion` ceiling and the narrowed `Judgment` bound.
pub const SCHEMA_VERSION_MAX: u64 = 9_007_199_254_740_991;
/// `2^53`, the inclusive ceiling of every revision member.
pub const REVISION_MAX: u64 = 9_007_199_254_740_992;

/// A `ParseIntError` for an in-range-typed value outside its wire domain.
///
/// The published `FromStr`/`TryFrom` error types are `ParseIntError`, so an
/// out-of-domain value fails with the same type as a malformed integer.
pub fn out_of_domain() -> ParseIntError {
    // shortcut: the empty-input parse error doubles as the bound error; its
    // Display text is not part of the contract. Upgrade if a distinguishable
    // bound error becomes observable.
    "".parse::<u64>().unwrap_err()
}

fn narrow_positive(value: u64) -> Option<NonZeroU64> {
    NonZeroU64::new(value).filter(|parsed| parsed.get() <= SCHEMA_VERSION_MAX)
}

fn revision_positive(value: u64) -> Option<NonZeroU64> {
    NonZeroU64::new(value).filter(|parsed| parsed.get() <= REVISION_MAX)
}

fn revision(value: u64) -> Option<u64> {
    (value <= REVISION_MAX).then_some(value)
}

/// `SchemaVersion`: `[1, 2^53-1]`.
pub fn parse_schema_version(value: &str) -> Result<NonZeroU64, ParseIntError> {
    narrow_positive(value.parse()?).ok_or_else(out_of_domain)
}

/// `Judgment.evidence[].revision` / `Judgment.target.revision`: `[1, 2^53-1]`.
pub fn parse_judgment_revision(value: &str) -> Result<NonZeroU64, ParseIntError> {
    parse_schema_version(value)
}

/// A positive per-object revision: `[1, 2^53]`.
pub fn parse_positive_revision(value: &str) -> Result<NonZeroU64, ParseIntError> {
    revision_positive(value.parse()?).ok_or_else(out_of_domain)
}

/// A revision member that admits `0`: `[0, 2^53]`.
pub fn parse_revision(value: &str) -> Result<u64, ParseIntError> {
    revision(value.parse()?).ok_or_else(out_of_domain)
}

/// Read one wire number as an integer. An integral float spelling names the
/// same wire number JavaScript reads (`9007199254740992.0` ≡ `2^53`); an
/// exact integer token keeps its own value, so `9007199254740993` is never
/// converted through `f64`.
struct ExactInteger;

impl Visitor<'_> for ExactInteger {
    type Value = u64;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a non-negative exact integer")
    }

    fn visit_u64<E: DeError>(self, value: u64) -> Result<u64, E> {
        Ok(value)
    }

    fn visit_i64<E: DeError>(self, value: i64) -> Result<u64, E> {
        u64::try_from(value).map_err(|_| E::custom("integer is negative"))
    }

    fn visit_f64<E: DeError>(self, value: f64) -> Result<u64, E> {
        if value.fract() == 0.0 && value >= 0.0 && value <= REVISION_MAX as f64 {
            Ok(value as u64)
        } else {
            Err(E::custom("number is not an exact non-negative integer"))
        }
    }
}

/// One required wire integer (see [`ExactInteger`]).
fn exact_integer<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    deserializer.deserialize_any(ExactInteger)
}

/// One optional wire integer: absent and `null` both read as `None`.
fn optional_exact_integer<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    struct OptionalExactInteger;

    impl<'de> Visitor<'de> for OptionalExactInteger {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an optional non-negative exact integer")
        }

        fn visit_none<E: DeError>(self) -> Result<Option<u64>, E> {
            Ok(None)
        }

        fn visit_unit<E: DeError>(self) -> Result<Option<u64>, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Option<u64>, D::Error> {
            exact_integer(deserializer).map(Some)
        }
    }

    deserializer.deserialize_option(OptionalExactInteger)
}

pub fn deserialize_schema_version<'de, D>(deserializer: D) -> Result<NonZeroU64, D::Error>
where
    D: Deserializer<'de>,
{
    narrow_positive(exact_integer(deserializer)?)
        .ok_or_else(|| D::Error::custom("schema_version outside [1, 2^53-1]"))
}

pub fn deserialize_judgment_revision<'de, D>(deserializer: D) -> Result<NonZeroU64, D::Error>
where
    D: Deserializer<'de>,
{
    narrow_positive(exact_integer(deserializer)?)
        .ok_or_else(|| D::Error::custom("revision outside [1, 2^53-1]"))
}

pub fn deserialize_optional_judgment_revision<'de, D>(
    deserializer: D,
) -> Result<Option<NonZeroU64>, D::Error>
where
    D: Deserializer<'de>,
{
    optional_exact_integer(deserializer)?
        .map(|raw| {
            narrow_positive(raw).ok_or_else(|| D::Error::custom("revision outside [1, 2^53-1]"))
        })
        .transpose()
}

pub fn deserialize_positive_revision<'de, D>(deserializer: D) -> Result<NonZeroU64, D::Error>
where
    D: Deserializer<'de>,
{
    revision_positive(exact_integer(deserializer)?)
        .ok_or_else(|| D::Error::custom("revision outside [1, 2^53]"))
}

pub fn deserialize_optional_positive_revision<'de, D>(
    deserializer: D,
) -> Result<Option<NonZeroU64>, D::Error>
where
    D: Deserializer<'de>,
{
    optional_exact_integer(deserializer)?
        .map(|raw| {
            revision_positive(raw).ok_or_else(|| D::Error::custom("revision outside [1, 2^53]"))
        })
        .transpose()
}

pub fn deserialize_revision<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    revision(exact_integer(deserializer)?)
        .ok_or_else(|| D::Error::custom("revision outside [0, 2^53]"))
}

pub fn deserialize_optional_revision<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    optional_exact_integer(deserializer)?
        .map(|raw| revision(raw).ok_or_else(|| D::Error::custom("revision outside [0, 2^53]")))
        .transpose()
}
