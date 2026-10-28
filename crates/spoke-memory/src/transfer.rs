//! Deterministic export/import transfer policy: RFC 8785 canonical digests,
//! §6.13 chunking, reassembly verification, reference translation, and the
//! §7.31 staged-import report.
//!
//! Bundles and chunks are handled as `serde_json::Value`: the §6.13 policy is
//! slicing arrays of already-serialized elements in a fixed field order and
//! re-digesting the result, while the generated `MemoryExportChunk` is a closed
//! header/tail enum of newtype-wrapped fields that cannot express "this chunk
//! object minus its own digest". Providers keep owning the wire types; the
//! engine works on their JSON projection. Reflection inputs stay typed
//! (`ReadSelection`, `ProvenanceFilter`) because they are pure value objects.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// Fixed array order for chunk partitioning and reassembly (§6.13, decision D8).
pub const BUNDLE_ARRAY_FIELDS: [&str; 7] = [
    "subjects",
    "records",
    "profiles",
    "reflections",
    "candidates",
    "receipt_summaries",
    "grants",
];

/// Logical bundle scalars carried in the ordinal-0 header, including the sealed
/// `digest` (§6.13 header fields) and `extensions` (decision D8).
pub const BUNDLE_HEADER_FIELDS: [&str; 9] = [
    "schema_version",
    "bundle_format_version",
    "memory_protocol_version",
    "export_id",
    "space_id",
    "created_at",
    "source",
    "digest",
    "extensions",
];

/// Header fields a chunk cannot be emitted without (§6.13 header schema).
const REQUIRED_HEADER_FIELDS: [&str; 8] = [
    "schema_version",
    "bundle_format_version",
    "memory_protocol_version",
    "export_id",
    "space_id",
    "created_at",
    "source",
    "digest",
];

/// Closed `counts` shape of `memory.imports.stage` (§7.31): grant totals live in
/// `grants_adopted`/`grants_dropped`, never in `counts`.
pub const STAGED_COUNT_FIELDS: [&str; 6] = [
    "subjects",
    "records",
    "candidates",
    "profiles",
    "reflections",
    "receipt_summaries",
];

/// Transfer-policy failures. They are provider-independent observations about
/// the artifact under verification, not protocol reject codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferError {
    NotAnObject(&'static str),
    MissingField(&'static str),
    UnexpectedType(&'static str),
    NonFiniteNumber,
    ChunkCountMismatch { declared: u64, actual: u64 },
    OrdinalSequence { expected: u64, found: u64 },
    ChunkMetadataMismatch(&'static str),
    ChunkDigestMismatch(u64),
    UnexpectedHeader(u64),
    BundleDigestMismatch,
    MissingMapping { kind: &'static str, id: String },
}

impl fmt::Display for TransferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject(field) => write!(formatter, "{field} is not a JSON object"),
            Self::MissingField(field) => write!(formatter, "{field} is missing"),
            Self::UnexpectedType(field) => write!(formatter, "{field} has an unexpected type"),
            Self::NonFiniteNumber => write!(formatter, "non-finite numbers have no JCS form"),
            Self::ChunkCountMismatch { declared, actual } => {
                write!(
                    formatter,
                    "chunk_count {declared} but {actual} chunks were supplied"
                )
            }
            Self::OrdinalSequence { expected, found } => {
                write!(
                    formatter,
                    "expected chunk ordinal {expected}, found {found}"
                )
            }
            Self::ChunkMetadataMismatch(field) => {
                write!(formatter, "chunks disagree on {field}")
            }
            Self::ChunkDigestMismatch(ordinal) => {
                write!(formatter, "chunk {ordinal} digest does not verify")
            }
            Self::UnexpectedHeader(ordinal) => {
                write!(
                    formatter,
                    "chunk {ordinal} carries a header; only ordinal 0 may"
                )
            }
            Self::BundleDigestMismatch => {
                write!(formatter, "reassembled bundle digest does not verify")
            }
            Self::MissingMapping { kind, id } => {
                write!(formatter, "{kind} {id} has no target assignment")
            }
        }
    }
}

impl std::error::Error for TransferError {}

/// RFC 8785 (JCS) canonical bytes of `value`.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, TransferError> {
    let mut out = String::new();
    write_canonical(value, &mut out)?;
    Ok(out.into_bytes())
}

fn write_canonical(value: &Value, out: &mut String) -> Result<(), TransferError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&canonical_number(number)?),
        Value::String(text) => write_canonical_string(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out)?;
            }
            out.push(']');
        }
        Value::Object(entries) => {
            out.push('{');
            let mut keys: Vec<&String> = entries.keys().collect();
            // RFC 8785 §3.2.3: property names sort by UTF-16 code units.
            keys.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_string(key, out);
                out.push(':');
                write_canonical(&entries[key.as_str()], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// RFC 8785 §3.2.2.2: only `"`, `\` and control characters below 0x20 escape.
fn write_canonical_string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{0009}' => out.push_str("\\t"),
            '\u{000a}' => out.push_str("\\n"),
            '\u{000c}' => out.push_str("\\f"),
            '\u{000d}' => out.push_str("\\r"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// RFC 8785 §3.2.2.3: every JSON number is an IEEE-754 double serialized with
/// ECMAScript `Number::toString` semantics, integers included.
fn canonical_number(number: &serde_json::Number) -> Result<String, TransferError> {
    let value = number.as_f64().ok_or(TransferError::NonFiniteNumber)?;
    if !value.is_finite() {
        return Err(TransferError::NonFiniteNumber);
    }
    if value == 0.0 {
        return Ok("0".to_string());
    }
    Ok(ecmascript_number(value))
}

/// ECMAScript `Number::toString` (RFC 8785 §3.2.2.3): shortest round-trip
/// digits (Rust's `{:e}`), plain decimal for `1e-6 <= |x| < 1e21`, exponential
/// with an explicit sign otherwise; `9007199254740993` becomes
/// `9007199254740992` because it parses to that double.
fn ecmascript_number(value: f64) -> String {
    let negative = value < 0.0;
    let formatted = format!("{:e}", value.abs());
    let (mantissa, exponent) = formatted
        .split_once('e')
        .expect("{:e} always writes an exponent");
    let digits: String = mantissa
        .chars()
        .filter(|character| *character != '.')
        .collect();
    let exponent: i32 = exponent.parse().expect("{:e} writes a decimal exponent");
    let length = digits.len() as i32;
    // `point` is the decimal point position relative to the digit string.
    let point = exponent + 1;

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if length <= point && point <= 21 {
        out.push_str(&digits);
        for _ in 0..(point - length) {
            out.push('0');
        }
    } else if 0 < point && point <= 21 {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    } else if -6 < point && point <= 0 {
        out.push_str("0.");
        for _ in 0..(-point) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if length > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if point - 1 >= 0 { '+' } else { '-' });
        out.push_str(&(point - 1).abs().to_string());
    }
    out
}

/// SHA-256 as 64 lowercase hex characters.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// §6.11 bundle digest: SHA-256 over JCS of the bundle minus `digest`.
pub fn bundle_digest(bundle: &Value) -> Result<String, TransferError> {
    digest_without(bundle, "digest", "bundle")
}

/// §6.13 chunk digest: SHA-256 over JCS of the chunk minus `chunk_digest`.
pub fn chunk_digest(chunk: &Value) -> Result<String, TransferError> {
    digest_without(chunk, "chunk_digest", "chunk")
}

fn digest_without(
    value: &Value,
    excluded: &str,
    label: &'static str,
) -> Result<String, TransferError> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or(TransferError::NotAnObject(label))?;
    object.remove(excluded);
    Ok(sha256_hex(&canonical_json_bytes(&Value::Object(object))?))
}

/// §6.13 partitioning: element-atomic slices in the fixed field order, header
/// only on ordinal 0, never splitting an element even when it exceeds the limit.
///
/// Presence is preserved: an array that is present but empty is emitted as `[]`
/// on ordinal 0, while an absent array is never emitted.
///
/// Sizing uses the canonical serialized chunk with a placeholder `chunk_count`
/// of the widest possible digit count, so a final chunk's `bytes` never exceeds
/// the measured value.
pub fn partition_bundle(bundle: &Value, chunk_limit: usize) -> Result<Vec<Value>, TransferError> {
    let object = bundle
        .as_object()
        .ok_or(TransferError::NotAnObject("bundle"))?;
    let export_id = object
        .get("export_id")
        .and_then(Value::as_str)
        .ok_or(TransferError::MissingField("export_id"))?
        .to_string();

    let recomputed_digest = bundle_digest(bundle)?;
    if let Some(declared) = object.get("digest") {
        if declared.as_str() != Some(recomputed_digest.as_str()) {
            return Err(TransferError::BundleDigestMismatch);
        }
    }
    for field in REQUIRED_HEADER_FIELDS {
        if !object.contains_key(field) {
            return Err(TransferError::MissingField(field));
        }
    }

    let header: Map<String, Value> = BUNDLE_HEADER_FIELDS
        .iter()
        .filter_map(|field| {
            let value = if *field == "digest" {
                json!(recomputed_digest)
            } else {
                object.get(*field)?.clone()
            };
            Some(((*field).to_string(), value))
        })
        .collect();

    let mut stream: Vec<(&'static str, &Value)> = Vec::new();
    let mut empty_present: Vec<&'static str> = Vec::new();
    for field in BUNDLE_ARRAY_FIELDS {
        match object.get(field) {
            None => {}
            Some(Value::Array(items)) if items.is_empty() => empty_present.push(field),
            Some(Value::Array(items)) => {
                stream.extend(items.iter().map(|item| (field, item)));
            }
            Some(_) => return Err(TransferError::UnexpectedType(field)),
        }
    }

    let widest_count = ((stream.len() as u64) + 1).to_string().len() as u32;
    let count_placeholder = 10u64.pow(widest_count).saturating_sub(1);

    let mut groups: Vec<Vec<(&'static str, &Value)>> = Vec::new();
    let mut current: Vec<(&'static str, &Value)> = Vec::new();
    for element in stream {
        current.push(element);
        if current.len() > 1 {
            let measured = build_chunk(
                &export_id,
                &recomputed_digest,
                &header,
                &empty_present,
                groups.len() as u64,
                count_placeholder,
                &current,
            )?;
            let bytes = measured["bytes"].as_u64().unwrap_or(u64::MAX);
            if bytes > chunk_limit as u64 {
                let last = current.pop().expect("the current chunk is non-empty");
                groups.push(std::mem::take(&mut current));
                current.push(last);
            }
        }
    }
    groups.push(current);

    let count = groups.len() as u64;
    groups
        .iter()
        .enumerate()
        .map(|(ordinal, group)| {
            build_chunk(
                &export_id,
                &recomputed_digest,
                &header,
                &empty_present,
                ordinal as u64,
                count,
                group,
            )
        })
        .collect()
}

fn build_chunk(
    export_id: &str,
    bundle_digest: &str,
    header: &Map<String, Value>,
    empty_present: &[&'static str],
    ordinal: u64,
    count: u64,
    group: &[(&'static str, &Value)],
) -> Result<Value, TransferError> {
    let mut chunk = Map::new();
    chunk.insert("export_id".to_string(), json!(export_id));
    chunk.insert("chunk_ordinal".to_string(), json!(ordinal));
    chunk.insert("chunk_count".to_string(), json!(count));
    chunk.insert("bundle_digest".to_string(), json!(bundle_digest));
    if ordinal == 0 {
        chunk.insert("header".to_string(), Value::Object(header.clone()));
    }
    for field in BUNDLE_ARRAY_FIELDS {
        let items: Vec<Value> = group
            .iter()
            .filter(|(element_field, _)| *element_field == field)
            .map(|(_, element)| (*element).clone())
            .collect();
        if !items.is_empty() {
            chunk.insert(field.to_string(), Value::Array(items));
        } else if ordinal == 0 && empty_present.contains(&field) {
            chunk.insert(field.to_string(), Value::Array(Vec::new()));
        }
    }
    // A fixed-width placeholder keeps the digest field's contribution to the
    // size constant while `bytes` settles.
    chunk.insert("chunk_digest".to_string(), json!("0".repeat(64)));

    let mut bytes = 0u64;
    loop {
        chunk.insert("bytes".to_string(), json!(bytes));
        let length = canonical_json_bytes(&Value::Object(chunk.clone()))?.len() as u64;
        if length == bytes {
            break;
        }
        bytes = length;
    }

    let digest = chunk_digest(&Value::Object(chunk.clone()))?;
    chunk.insert("chunk_digest".to_string(), json!(digest));
    Ok(Value::Object(chunk))
}

/// Outcome of the verified reassembly path: the reassembled bundle (when the
/// header allowed building one), whether its digest matched, and every failure.
struct Reassembly {
    bundle: Option<Value>,
    bundle_digest_ok: bool,
    errors: Vec<TransferError>,
}

fn reassemble_verified(chunks: &[Value]) -> Reassembly {
    let mut errors = Vec::new();
    if chunks.is_empty() {
        errors.push(TransferError::MissingField("chunks"));
        return Reassembly {
            bundle: None,
            bundle_digest_ok: false,
            errors,
        };
    }

    let mut ordered: Vec<(u64, &Value)> = chunks
        .iter()
        .map(|chunk| {
            let ordinal = chunk
                .get("chunk_ordinal")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX);
            (ordinal, chunk)
        })
        .collect();
    ordered.sort_by_key(|(ordinal, _)| *ordinal);
    let total = ordered.len() as u64;
    for (index, (ordinal, _)) in ordered.iter().enumerate() {
        if *ordinal != index as u64 {
            errors.push(TransferError::OrdinalSequence {
                expected: index as u64,
                found: *ordinal,
            });
        }
    }

    let mut header: Option<Map<String, Value>> = None;
    let mut present: BTreeSet<&'static str> = BTreeSet::new();
    let mut slices: BTreeMap<&'static str, Vec<Value>> = BUNDLE_ARRAY_FIELDS
        .iter()
        .map(|field| (*field, Vec::new()))
        .collect();

    for (index, (ordinal, chunk)) in ordered.iter().enumerate() {
        let Some(object) = chunk.as_object() else {
            errors.push(TransferError::NotAnObject("chunk"));
            continue;
        };
        match object.get("chunk_count").and_then(Value::as_u64) {
            Some(declared) if declared != total => {
                errors.push(TransferError::ChunkCountMismatch {
                    declared,
                    actual: total,
                });
            }
            Some(_) => {}
            None => errors.push(TransferError::MissingField("chunk_count")),
        }
        match chunk_digest(chunk) {
            Ok(expected) => {
                let recorded = object
                    .get("chunk_digest")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if recorded != expected {
                    errors.push(TransferError::ChunkDigestMismatch(*ordinal));
                }
            }
            Err(error) => errors.push(error),
        }
        match (index, object.get("header")) {
            (0, Some(Value::Object(fields))) => header = Some(fields.clone()),
            (0, Some(_)) => errors.push(TransferError::UnexpectedType("header")),
            (0, None) => errors.push(TransferError::MissingField("header")),
            (_, Some(_)) => errors.push(TransferError::UnexpectedHeader(*ordinal)),
            (_, None) => {}
        }
        for field in BUNDLE_ARRAY_FIELDS {
            if let Some(items) = object.get(field) {
                present.insert(field);
                match items.as_array() {
                    Some(items) => slices
                        .get_mut(field)
                        .expect("every array field is tracked")
                        .extend(items.iter().cloned()),
                    None => errors.push(TransferError::UnexpectedType(field)),
                }
            }
        }
    }

    let Some(header) = header else {
        return Reassembly {
            bundle: None,
            bundle_digest_ok: false,
            errors,
        };
    };
    let header_export_id = header.get("export_id").and_then(Value::as_str);
    let header_digest = header
        .get("digest")
        .and_then(Value::as_str)
        .map(str::to_owned);
    for (_, chunk) in &ordered {
        if chunk.get("export_id").and_then(Value::as_str) != header_export_id {
            errors.push(TransferError::ChunkMetadataMismatch("export_id"));
        }
        if chunk.get("bundle_digest").and_then(Value::as_str) != header_digest.as_deref() {
            errors.push(TransferError::ChunkMetadataMismatch("bundle_digest"));
        }
    }

    let mut bundle = header;
    for (field, items) in slices {
        if present.contains(field) {
            bundle.insert(field.to_string(), Value::Array(items));
        }
    }
    let bundle = Value::Object(bundle);
    let bundle_digest_ok = match (header_digest, bundle_digest(&bundle)) {
        (Some(declared), Ok(recomputed)) => {
            if declared == recomputed {
                true
            } else {
                errors.push(TransferError::BundleDigestMismatch);
                false
            }
        }
        (Some(_), Err(error)) => {
            errors.push(error);
            false
        }
        (None, _) => {
            errors.push(TransferError::MissingField("header.digest"));
            false
        }
    };

    Reassembly {
        bundle: Some(bundle),
        bundle_digest_ok,
        errors,
    }
}

/// Reassemble the logical bundle from verified chunks (§6.13, §6.11).
///
/// Every per-chunk digest, the shared `chunk_count`, ordinal continuity, header
/// placement (ordinal 0 only) and the header's sealed `digest` are checked; the
/// first failure is returned. Use [`verify_reassembly`] when all failures are
/// needed.
pub fn reassemble(chunks: &[Value]) -> Result<Value, TransferError> {
    let Reassembly { bundle, errors, .. } = reassemble_verified(chunks);
    if let Some(error) = errors.into_iter().next() {
        return Err(error);
    }
    bundle.ok_or(TransferError::MissingField("header"))
}

/// Outcome of chunk verification (§7.31 `staged_report.verification`).
pub struct Verification {
    pub bundle_digest_ok: bool,
    pub errors: Vec<String>,
}

/// Verify per-chunk digests, chunk structure and the reassembled bundle digest;
/// never panics on malformed input — every failure becomes a message in
/// `errors`. Acceptance requires both `bundle_digest_ok` and an empty `errors`.
pub fn verify_reassembly(chunks: &[Value]) -> Verification {
    let reassembly = reassemble_verified(chunks);
    Verification {
        bundle_digest_ok: reassembly.bundle_digest_ok,
        errors: reassembly.errors.iter().map(ToString::to_string).collect(),
    }
}

/// §7.31 reference translation maps (source id → target id).
#[derive(Debug, Default, Clone)]
pub struct TranslationMaps {
    pub subject_id_map: BTreeMap<String, String>,
    pub record_id_map: BTreeMap<String, String>,
    pub candidate_id_map: BTreeMap<String, String>,
}

/// Build the translation maps for a reassembled bundle (§7.31, §12.3).
///
/// Subject ids are preserved unless renamed. Record and candidate ids are
/// reassigned by the target, so a missing assignment is reported as an error
/// and left out of the map — source ids are never presented as target ids.
pub fn build_translation_maps(
    bundle: &Value,
    subject_renames: &BTreeMap<String, String>,
    record_ids: &BTreeMap<String, String>,
    candidate_ids: &BTreeMap<String, String>,
) -> (TranslationMaps, Vec<TransferError>) {
    let mut errors = Vec::new();
    let maps = TranslationMaps {
        subject_id_map: identity_map(
            bundle,
            "subjects",
            "subject_id",
            "subject",
            subject_renames,
            false,
            &mut errors,
        ),
        record_id_map: identity_map(
            bundle,
            "records",
            "record_id",
            "record",
            record_ids,
            true,
            &mut errors,
        ),
        candidate_id_map: identity_map(
            bundle,
            "candidates",
            "candidate_id",
            "candidate",
            candidate_ids,
            true,
            &mut errors,
        ),
    };
    (maps, errors)
}

fn identity_map(
    bundle: &Value,
    field: &'static str,
    id_field: &'static str,
    kind: &'static str,
    overrides: &BTreeMap<String, String>,
    require_assignment: bool,
    errors: &mut Vec<TransferError>,
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let items = bundle
        .get(field)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for item in items {
        let Some(source) = item.get(id_field).and_then(Value::as_str) else {
            errors.push(TransferError::MissingField(id_field));
            continue;
        };
        match overrides.get(source) {
            Some(target) => {
                map.insert(source.to_string(), target.clone());
            }
            None if require_assignment => errors.push(TransferError::MissingMapping {
                kind,
                id: source.to_string(),
            }),
            None => {
                map.insert(source.to_string(), source.to_string());
            }
        }
    }
    map
}

/// Remap a candidate's `consumed_by.record_ids` through the record map (§7.31).
/// A reference without a target assignment is rejected, never left pointing at
/// a source id.
pub fn remap_candidate_consumed_by(
    candidate: &Value,
    maps: &TranslationMaps,
) -> Result<Value, TransferError> {
    let mut candidate = candidate.clone();
    let object = candidate
        .as_object_mut()
        .ok_or(TransferError::NotAnObject("candidate"))?;
    if let Some(consumed_by) = object.get_mut("consumed_by") {
        let consumed = consumed_by
            .as_object_mut()
            .ok_or(TransferError::UnexpectedType("consumed_by"))?;
        if let Some(record_ids) = consumed.get_mut("record_ids") {
            let items = record_ids
                .as_array()
                .ok_or(TransferError::UnexpectedType("record_ids"))?;
            let remapped: Vec<Value> = items
                .iter()
                .map(|item| {
                    let source = item
                        .as_str()
                        .ok_or(TransferError::UnexpectedType("record_ids"))?;
                    match maps.record_id_map.get(source) {
                        Some(target) => Ok(json!(target)),
                        None => Err(TransferError::MissingMapping {
                            kind: "record",
                            id: source.to_string(),
                        }),
                    }
                })
                .collect::<Result<_, TransferError>>()?;
            *record_ids = Value::Array(remapped);
        }
    }
    Ok(candidate)
}

/// Grants adopted by explicit `adopt_grants`: returns the adopted summaries and
/// the number that were dropped (§7.31, MEM-R025).
pub fn adopt_grants(bundle: &Value, adopt: &[&str]) -> Result<(Vec<Value>, usize), TransferError> {
    let grants = bundle
        .get("grants")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut adopted = Vec::new();
    let mut dropped = 0;
    for grant in grants {
        let grant_id = grant
            .get("grant_id")
            .and_then(Value::as_str)
            .ok_or(TransferError::MissingField("grant_id"))?;
        if adopt.contains(&grant_id) {
            adopted.push(grant.clone());
        } else {
            dropped += 1;
        }
    }
    Ok((adopted, dropped))
}

/// §7.31 staged outcome: the reassembled bundle (when structure allowed it), the
/// reference translation maps, and one verification record covering chunk,
/// bundle and translation failures.
pub struct StagedOutcome {
    pub bundle: Option<Value>,
    pub maps: TranslationMaps,
    pub verification: Verification,
}

/// Verify staged chunks and reference translation in one pass.
pub fn verify_staging(
    chunks: &[Value],
    subject_renames: &BTreeMap<String, String>,
    record_ids: &BTreeMap<String, String>,
    candidate_ids: &BTreeMap<String, String>,
) -> StagedOutcome {
    let reassembly = reassemble_verified(chunks);
    let mut errors: Vec<String> = reassembly.errors.iter().map(ToString::to_string).collect();
    let maps = match &reassembly.bundle {
        Some(bundle) => {
            let (maps, translation_errors) =
                build_translation_maps(bundle, subject_renames, record_ids, candidate_ids);
            errors.extend(translation_errors.iter().map(ToString::to_string));
            maps
        }
        None => TranslationMaps::default(),
    };
    StagedOutcome {
        bundle: reassembly.bundle,
        maps,
        verification: Verification {
            bundle_digest_ok: reassembly.bundle_digest_ok,
            errors,
        },
    }
}

/// §7.31 staged report: the closed six-property `counts` shape, translation
/// maps, grant adoption totals, and the verification outcome.
pub fn staged_report(
    bundle: &Value,
    maps: &TranslationMaps,
    grants_adopted: usize,
    grants_dropped: usize,
    verification: &Verification,
) -> Result<Value, TransferError> {
    let object = bundle
        .as_object()
        .ok_or(TransferError::NotAnObject("bundle"))?;
    let counts: Map<String, Value> = STAGED_COUNT_FIELDS
        .iter()
        .map(|field| {
            let count = object
                .get(*field)
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            ((*field).to_string(), json!(count))
        })
        .collect();
    Ok(json!({
        "counts": counts,
        "subject_id_map": maps.subject_id_map,
        "record_id_map": maps.record_id_map,
        "candidate_id_map": maps.candidate_id_map,
        "grants_adopted": grants_adopted,
        "grants_dropped": grants_dropped,
        "verification": {
            "bundle_digest_ok": verification.bundle_digest_ok,
            "errors": verification.errors,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(id: &str) -> Value {
        json!({
            "schema_version": 1,
            "subject_id": id,
            "space_id": "spc_local",
            "created_at": "2026-10-28T09:00:00Z",
            "revision": 1,
            "extensions": {},
        })
    }

    fn record(id: &str, text: &str) -> Value {
        json!({
            "schema_version": 1,
            "record_id": id,
            "space_id": "spc_local",
            "subject_id": "subj_mira",
            "kind": "fact",
            "content": {"text": text},
            "scope": {"visibility": "shared"},
            "provenance": {
                "source_context_ids": [],
                "anchors": [],
                "captured_by": "prn_host",
                "captured_at": "2026-10-28T09:00:00Z",
            },
            "created_at": "2026-10-28T09:00:00Z",
            "updated_at": "2026-10-28T09:00:00Z",
            "revision": 1,
            "extensions": {},
        })
    }

    fn seal(mut bundle: Value) -> Value {
        let digest = bundle_digest(&bundle).expect("bundle digest computes");
        bundle
            .as_object_mut()
            .expect("bundle is an object")
            .insert("digest".to_string(), Value::String(digest));
        bundle
    }

    fn sealed_bundle() -> Value {
        seal(json!({
            "schema_version": 1,
            "bundle_format_version": 1,
            "memory_protocol_version": 1,
            "export_id": "exp_1",
            "space_id": "spc_local",
            "created_at": "2026-10-28T09:00:00Z",
            "source": {
                "authority_host_id": "host_a",
                "authority_generation": 3,
                "engine_policy_version": "spoke-memory/1.0.0-deterministic",
            },
            "subjects": [subject("subj_mira"), subject("subj_ghost")],
            "records": [record("rec_a", "alpha"), record("rec_b", "bravo"), record("rec_c", "charlie")],
            "profiles": [],
            "reflections": [],
            "candidates": [{
                "schema_version": 1,
                "candidate_id": "cand_1",
                "space_id": "spc_local",
                "subject_id": "subj_mira",
                "capture_scope": {"visibility": "shared"},
                "proposed": {"kind": "fact"},
                "provenance": {
                    "source_context_ids": [],
                    "anchors": [],
                    "captured_by": "prn_host",
                    "captured_at": "2026-10-28T09:00:00Z",
                },
                "classification": {"disposition": "promote", "rationale_codes": []},
                "status": "consumed",
                "consumed_by": {
                    "op": "memory.candidates.review",
                    "receipt_id": "rcp_1",
                    "outcome": "promoted",
                    "record_ids": ["rec_a"],
                },
                "content_digest": "0000000000000000000000000000000000000000000000000000000000000000",
                "created_at": "2026-10-28T09:00:00Z",
                "revision": 1,
                "extensions": {},
            }],
            "receipt_summaries": [{
                "receipt_id": "rcp_1",
                "op": "memory.candidates.review",
                "requester_principal_ref": "prn_label",
                "payload_digest": "1111111111111111111111111111111111111111111111111111111111111111",
                "status": "committed",
                "created_at": "2026-10-28T09:00:00Z",
            }],
            "grants": [
                {
                    "grant_id": "grt_1",
                    "principal_ref": "prn_label",
                    "capabilities": ["read"],
                    "revision": 1,
                },
                {
                    "grant_id": "grt_2",
                    "principal_ref": "prn_label",
                    "capabilities": ["admin"],
                    "revision": 1,
                },
            ],
            "extensions": {"spoke_note": {"build": 7}},
        }))
    }

    fn complete_assignments() -> (BTreeMap<String, String>, BTreeMap<String, String>) {
        let mut records = BTreeMap::new();
        records.insert("rec_a".to_string(), "rec_a_target".to_string());
        records.insert("rec_b".to_string(), "rec_b_target".to_string());
        records.insert("rec_c".to_string(), "rec_c_target".to_string());
        let mut candidates = BTreeMap::new();
        candidates.insert("cand_1".to_string(), "cand_1_target".to_string());
        (records, candidates)
    }

    /// Locate the closed `Counts` definition inside the consumer schema.
    fn schema_counts(value: &Value) -> Option<(Vec<String>, Vec<String>)> {
        match value {
            Value::Object(fields) => {
                if fields.get("title").and_then(Value::as_str) == Some("Counts") {
                    let required = fields
                        .get("required")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut properties = fields
                        .get("properties")
                        .and_then(Value::as_object)
                        .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    properties.sort();
                    return Some((required, properties));
                }
                fields.values().find_map(schema_counts)
            }
            Value::Array(items) => items.iter().find_map(schema_counts),
            _ => None,
        }
    }

    #[test]
    fn canonical_json_sorts_keys_and_escapes_strings() {
        let value = json!({
            "b": 1,
            "a": [true, null],
            "z": {"é": 1, "e": 2},
            "\u{1}": 0,
        });
        let canonical = String::from_utf8(canonical_json_bytes(&value).expect("canonicalizes"))
            .expect("canonical form is UTF-8");
        assert_eq!(
            canonical,
            r#"{"\u0001":0,"a":[true,null],"b":1,"z":{"e":2,"é":1}}"#
        );

        let value = json!({"s": "quote\" back\\slash\ttab\nnewline\u{1f}"});
        let canonical = String::from_utf8(canonical_json_bytes(&value).expect("canonicalizes"))
            .expect("canonical form is UTF-8");
        assert_eq!(
            canonical,
            r#"{"s":"quote\" back\\slash\ttab\nnewline\u001f"}"#
        );
    }

    #[test]
    fn canonical_json_matches_rfc8785_number_vectors() {
        // RFC 8785 Appendix B sample numbers and the §3.2.2.3 boundary cases.
        let value = json!([
            333333333.33333329,
            1e30,
            4.50,
            2e-3,
            0.000000000000000000000000001,
            1e21,
            1e-6,
            1e-7,
            1e20,
            1e-27
        ]);
        let canonical = String::from_utf8(canonical_json_bytes(&value).expect("canonicalizes"))
            .expect("canonical form is UTF-8");
        assert_eq!(
            canonical,
            "[333333333.3333333,1e+30,4.5,0.002,1e-27,1e+21,0.000001,1e-7,100000000000000000000,1e-27]"
        );
        // sha256 of the canonical string above, computed independently in Python
        // with an ECMAScript number formatter.
        assert_eq!(
            sha256_hex(canonical.as_bytes()),
            "66ffdccf28e737e8a4d685c1a3ef0ea0cfe0446f1869aff90619e0929b3c3501"
        );
    }

    #[test]
    fn canonical_json_serializes_integers_as_ecmascript_doubles() {
        let value = json!([
            9007199254740993u64,
            9007199254740992u64,
            9007199254740994u64
        ]);
        let canonical = String::from_utf8(canonical_json_bytes(&value).expect("canonicalizes"))
            .expect("canonical form is UTF-8");
        assert_eq!(
            canonical,
            "[9007199254740992,9007199254740992,9007199254740994]"
        );
        // sha256 of the canonical string above, computed independently in Python.
        assert_eq!(
            sha256_hex(canonical.as_bytes()),
            "a3c9dc6fdb3c0601d1a989d64ffe6391e4f045dea47d2befeff7733892e95f8c"
        );
    }

    #[test]
    fn chunk_round_trip_preserves_the_bundle() {
        let bundle = sealed_bundle();
        let chunks = partition_bundle(&bundle, 900).expect("partitions");
        assert!(chunks.len() > 1, "fixture must span multiple chunks");
        assert_eq!(chunks[0]["chunk_ordinal"], json!(0));
        assert_eq!(chunks[0]["header"]["digest"], bundle["digest"]);
        assert_eq!(chunks[0]["chunk_count"], json!(chunks.len() as u64));
        for (ordinal, chunk) in chunks.iter().enumerate().skip(1) {
            assert_eq!(chunk["chunk_ordinal"], json!(ordinal as u64));
            assert!(chunk.get("header").is_none(), "header is ordinal-0 only");
            assert_eq!(chunk["export_id"], json!("exp_1"));
            assert_eq!(chunk["bundle_digest"], bundle["digest"]);
        }

        let rebuilt = reassemble(&chunks).expect("reassembles");
        assert_eq!(rebuilt, bundle);
        let verification = verify_reassembly(&chunks);
        assert!(verification.bundle_digest_ok);
        assert!(verification.errors.is_empty());
    }

    #[test]
    fn absent_arrays_stay_absent_and_present_empty_arrays_stay_present() {
        let mut bundle = sealed_bundle();
        bundle
            .as_object_mut()
            .expect("bundle is an object")
            .remove("reflections");
        let bundle = seal(bundle);
        let rebuilt =
            reassemble(&partition_bundle(&bundle, 900).expect("partitions")).expect("reassembles");
        assert!(
            rebuilt.get("reflections").is_none(),
            "an absent optional array must not be injected"
        );
        assert_eq!(rebuilt, bundle);

        let bundle = sealed_bundle();
        assert_eq!(bundle["reflections"], json!([]));
        let rebuilt =
            reassemble(&partition_bundle(&bundle, 900).expect("partitions")).expect("reassembles");
        assert_eq!(
            rebuilt["reflections"],
            json!([]),
            "a present empty array must round-trip as present"
        );
        assert_eq!(rebuilt, bundle);
    }

    #[test]
    fn verification_detects_payload_and_chunk_tampering() {
        let bundle = sealed_bundle();
        let mut chunks = partition_bundle(&bundle, 900).expect("partitions");

        // Tamper with a payload field: the bundle digest no longer verifies.
        let last = chunks.len() - 1;
        let tampered = chunks[last].as_object_mut().expect("chunk is an object");
        for field in BUNDLE_ARRAY_FIELDS {
            if let Some(Value::Array(items)) = tampered.get_mut(field) {
                if !items.is_empty() {
                    items[0] = json!({"tampered": true});
                    break;
                }
            }
        }
        let verification = verify_reassembly(&chunks);
        assert!(!verification.bundle_digest_ok);
        assert!(!verification.errors.is_empty());

        // Tamper with a chunk digest: the per-chunk check fails while the
        // payload's bundle digest still verifies (it is reported through errors).
        let mut chunks = partition_bundle(&bundle, 900).expect("partitions");
        chunks[0]
            .as_object_mut()
            .expect("chunk is an object")
            .insert("chunk_digest".to_string(), json!("0".repeat(64)));
        let verification = verify_reassembly(&chunks);
        assert!(verification.bundle_digest_ok);
        assert_eq!(verification.errors.len(), 1);
        assert!(verification
            .errors
            .iter()
            .any(|error| error.contains("chunk 0")));
    }

    #[test]
    fn reassembly_rejects_inconsistent_chunk_structure() {
        let bundle = sealed_bundle();
        let chunks = partition_bundle(&bundle, 900).expect("partitions");
        assert!(chunks.len() > 2, "fixture must have tail chunks");

        // A contradictory chunk_count on a tail chunk.
        let mut broken = chunks.clone();
        broken[1]
            .as_object_mut()
            .expect("chunk is an object")
            .insert("chunk_count".to_string(), json!(chunks.len() as u64 + 1));
        let verification = verify_reassembly(&broken);
        assert!(verification
            .errors
            .iter()
            .any(|error| error.contains("chunk_count")));
        assert!(reassemble(&broken).is_err());

        // A header on a chunk other than ordinal 0.
        let mut broken = chunks.clone();
        let header = broken[0]["header"].clone();
        broken[1]
            .as_object_mut()
            .expect("chunk is an object")
            .insert("header".to_string(), header);
        let verification = verify_reassembly(&broken);
        assert!(verification
            .errors
            .iter()
            .any(|error| error.contains("only ordinal 0")));

        // A contradictory sealed digest in the header.
        let mut broken = chunks.clone();
        broken[0]["header"]
            .as_object_mut()
            .expect("header is an object")
            .insert("digest".to_string(), json!("f".repeat(64)));
        let verification = verify_reassembly(&broken);
        assert!(!verification.bundle_digest_ok);
        assert!(verification
            .errors
            .iter()
            .any(|error| error.contains("bundle digest")));
    }

    #[test]
    fn oversized_element_gets_its_own_chunk() {
        let mut bundle = sealed_bundle();
        bundle
            .as_object_mut()
            .expect("bundle is an object")
            .get_mut("records")
            .and_then(Value::as_array_mut)
            .expect("records is an array")
            .push(record("rec_big", &"x".repeat(4000)));
        let bundle = seal(bundle);

        let limit = 900;
        let chunks = partition_bundle(&bundle, limit).expect("partitions");
        let big_chunk = chunks
            .iter()
            .find(|chunk| {
                chunk["records"].as_array().is_some_and(|records| {
                    records.iter().any(|r| r["record_id"] == json!("rec_big"))
                })
            })
            .expect("the oversized record's chunk exists");
        assert_eq!(
            big_chunk["records"]
                .as_array()
                .expect("records array")
                .len(),
            1,
            "the oversized element is never split"
        );
        assert!(
            big_chunk["bytes"].as_u64().expect("bytes is an integer") > limit as u64,
            "the oversized chunk flags itself through bytes"
        );
        assert_eq!(reassemble(&chunks).expect("reassembles"), bundle);
    }

    #[test]
    fn translation_maps_remap_references() {
        let bundle = sealed_bundle();
        let mut renames = BTreeMap::new();
        renames.insert("subj_ghost".to_string(), "subj_ghost_target".to_string());
        let (records, candidates) = complete_assignments();

        let (maps, errors) = build_translation_maps(&bundle, &renames, &records, &candidates);
        assert!(errors.is_empty(), "complete assignments are accepted");
        assert_eq!(maps.subject_id_map["subj_mira"], "subj_mira");
        assert_eq!(maps.subject_id_map["subj_ghost"], "subj_ghost_target");
        assert_eq!(maps.record_id_map["rec_a"], "rec_a_target");
        assert_eq!(maps.record_id_map["rec_b"], "rec_b_target");
        assert_eq!(maps.candidate_id_map["cand_1"], "cand_1_target");

        let candidate = &bundle["candidates"][0];
        let remapped = remap_candidate_consumed_by(candidate, &maps).expect("remaps");
        assert_eq!(
            remapped["consumed_by"]["record_ids"],
            json!(["rec_a_target"])
        );
    }

    #[test]
    fn missing_translations_are_reported_never_patched() {
        let bundle = sealed_bundle();
        let mut records = BTreeMap::new();
        records.insert("rec_a".to_string(), "rec_a_target".to_string());
        let (_, candidates) = complete_assignments();

        let (maps, errors) =
            build_translation_maps(&bundle, &BTreeMap::new(), &records, &candidates);
        assert_eq!(maps.record_id_map.len(), 1);
        assert!(
            !maps.record_id_map.contains_key("rec_b"),
            "an unassigned record id must not fall back to its source id"
        );
        assert!(errors.iter().any(|error| *error
            == TransferError::MissingMapping {
                kind: "record",
                id: "rec_b".to_string()
            }));
        assert!(errors.iter().any(|error| *error
            == TransferError::MissingMapping {
                kind: "record",
                id: "rec_c".to_string()
            }));

        // A reference translation through an incomplete map is rejected.
        let mut shallow = bundle["candidates"][0].clone();
        shallow["consumed_by"]["record_ids"] = json!(["rec_b"]);
        assert!(remap_candidate_consumed_by(&shallow, &maps).is_err());
    }

    #[test]
    fn verify_staging_collects_translation_failures() {
        let bundle = sealed_bundle();
        let chunks = partition_bundle(&bundle, 900).expect("partitions");
        let mut records = BTreeMap::new();
        records.insert("rec_a".to_string(), "rec_a_target".to_string());
        let (_, candidates) = complete_assignments();

        let outcome = verify_staging(&chunks, &BTreeMap::new(), &records, &candidates);
        assert!(outcome.verification.bundle_digest_ok);
        assert!(outcome
            .verification
            .errors
            .iter()
            .any(|error| error.contains("record rec_b")));
        assert!(outcome.bundle.is_some());

        let (all_records, all_candidates) = complete_assignments();
        let outcome = verify_staging(&chunks, &BTreeMap::new(), &all_records, &all_candidates);
        assert!(outcome.verification.bundle_digest_ok);
        assert!(outcome.verification.errors.is_empty());
    }

    #[test]
    fn grant_adoption_splits_adopted_and_dropped() {
        let bundle = sealed_bundle();
        let (adopted, dropped) = adopt_grants(&bundle, &["grt_2"]).expect("adopts");
        assert_eq!(adopted.len(), 1);
        assert_eq!(adopted[0]["grant_id"], json!("grt_2"));
        assert_eq!(dropped, 1);
    }

    #[test]
    fn staged_report_matches_the_consumer_schema() {
        let bundle = sealed_bundle();
        let (maps, errors) = build_translation_maps(
            &bundle,
            &BTreeMap::new(),
            &complete_assignments().0,
            &complete_assignments().1,
        );
        assert!(errors.is_empty());
        let verification = verify_reassembly(&partition_bundle(&bundle, 900).expect("partitions"));
        let report = staged_report(&bundle, &maps, 1, 1, &verification).expect("report builds");

        let schema: Value = serde_json::from_str(include_str!(
            "../../../schemas/ops/memory-imports-stage-response.schema.json"
        ))
        .expect("consumer schema parses");
        let (required, properties) =
            schema_counts(&schema).expect("schema declares a Counts shape");

        let counts = report["counts"].as_object().expect("counts is an object");
        let mut produced: Vec<String> = counts.keys().cloned().collect();
        produced.sort();
        assert_eq!(
            produced, properties,
            "counts exposes exactly the schema's properties"
        );
        let mut required_sorted = required;
        required_sorted.sort();
        assert_eq!(
            produced, required_sorted,
            "every schema-required count is present"
        );
        assert_eq!(
            counts["subjects"],
            json!(2),
            "counts report the bundled element totals"
        );
        assert_eq!(counts["records"], json!(3));
        assert_eq!(counts["candidates"], json!(1));
        assert_eq!(counts["receipt_summaries"], json!(1));
        assert!(
            !counts.contains_key("grants"),
            "grant totals are separate fields"
        );

        assert_eq!(report["grants_adopted"], json!(1));
        assert_eq!(report["grants_dropped"], json!(1));
        assert_eq!(report["verification"]["bundle_digest_ok"], json!(true));
        assert_eq!(report["verification"]["errors"], json!([]));
    }
}
