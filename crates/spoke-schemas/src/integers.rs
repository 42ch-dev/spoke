//! Bounded wire integers (spoke-data-model §Shared wire scalars).
//!
//! `SchemaVersion` and the revision family keep their published Rust types
//! (`NonZeroU64` / `u64`); these helpers enforce the schema ceiling on the
//! serde, `FromStr` and `TryFrom` entry points. The inclusive revision bound
//! `[0, 2^53]` is asserted on the parsed integer value.

use serde::de::{Deserializer, Error as DeError};
use serde::Deserialize;
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

pub fn deserialize_schema_version<'de, D>(deserializer: D) -> Result<NonZeroU64, D::Error>
where
    D: Deserializer<'de>,
{
    narrow_positive(u64::deserialize(deserializer)?)
        .ok_or_else(|| D::Error::custom("schema_version outside [1, 2^53-1]"))
}

pub fn deserialize_judgment_revision<'de, D>(deserializer: D) -> Result<NonZeroU64, D::Error>
where
    D: Deserializer<'de>,
{
    narrow_positive(u64::deserialize(deserializer)?)
        .ok_or_else(|| D::Error::custom("revision outside [1, 2^53-1]"))
}

pub fn deserialize_optional_judgment_revision<'de, D>(
    deserializer: D,
) -> Result<Option<NonZeroU64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<u64>::deserialize(deserializer)?
        .map(|raw| {
            narrow_positive(raw).ok_or_else(|| D::Error::custom("revision outside [1, 2^53-1]"))
        })
        .transpose()
}

pub fn deserialize_positive_revision<'de, D>(deserializer: D) -> Result<NonZeroU64, D::Error>
where
    D: Deserializer<'de>,
{
    revision_positive(u64::deserialize(deserializer)?)
        .ok_or_else(|| D::Error::custom("revision outside [1, 2^53]"))
}

pub fn deserialize_optional_positive_revision<'de, D>(
    deserializer: D,
) -> Result<Option<NonZeroU64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<u64>::deserialize(deserializer)?
        .map(|raw| {
            revision_positive(raw).ok_or_else(|| D::Error::custom("revision outside [1, 2^53]"))
        })
        .transpose()
}

pub fn deserialize_revision<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    revision(u64::deserialize(deserializer)?)
        .ok_or_else(|| D::Error::custom("revision outside [0, 2^53]"))
}

pub fn deserialize_optional_revision<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<u64>::deserialize(deserializer)?
        .map(|raw| revision(raw).ok_or_else(|| D::Error::custom("revision outside [0, 2^53]")))
        .transpose()
}
