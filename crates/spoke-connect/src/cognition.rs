//! Raw `spoke-cognition` requirement diagnosis (consumer contract §2.2 /
//! data contract §5.2).
//!
//! One conclusion, three integration sites: the responder evaluates it on the
//! RAW payload before its typed `Scope` decode, the generic node dispatch
//! evaluates it before the configured handler, and the wire spelling
//! [`WIRE_INVALID_INPUT`] is stamped by both. Keeping the diagnosis here — not
//! in a transport module — is what lets the operations-free node path and the
//! operations-backed responder reach the same verdict.
//!
//! The closed grammar itself stays C5's `validate_cognition_filter` /
//! `validate_cognition_selection`, which also name the failing member relative
//! to the validated value; this module only prefixes the carrier location, so
//! no second predicate grammar exists here.

use serde_json::{Map, Value};
use spoke_operations::{validate_cognition_filter, validate_cognition_selection, SpokeResult};

use crate::core::cognition_requirement_field;

/// The shared negotiation/requirement vectors — the SAME file the TypeScript
/// suite consumes, so both languages agree on the host manifests, payloads
/// and exact rejection fields.
#[cfg(test)]
pub(crate) const COGNITION_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/spoke-connect-ts/tests/fixtures/cognition-negotiation.json"
));

/// Wire spelling of a dispatch-layer validation verdict (consumer contract
/// §2.2). The ops protocol spells the same internal code `INVALID_INPUT`
/// (`SpokeRejectCode::as_str`) on its envelopes; a rejection taken at the
/// pre-handler boundary carries this lowercase form instead, beside the other
/// dispatch codes (`op_unsupported`, `capability_missing`, `auth_failed`).
pub(crate) const WIRE_INVALID_INPUT: &str = "invalid_input";

/// Message for the closed-grammar unsupported requirement (consumer contract
/// §2.2 example envelope; parity with TS).
const UNSUPPORTED_COGNITION_MESSAGE: &str = "Unsupported cognition requirement";
/// Message for an unavailable requested source view (recorded-at-time
/// placement is a Memory source view; parity with TS and the C6 KE path).
const UNAVAILABLE_COGNITION_VIEW_MESSAGE: &str =
    "Unsupported cognition requirement: recorded-at-time source is unavailable";

/// The request's unsupported cognition requirement, if any: the envelope
/// message plus the `details` object (`reason` + the actual `field` path).
///
/// Runs on the RAW payload — before any typed decode could drop or rename an
/// unknown predicate — and covers exactly the op's declared carrier
/// ([`cognition_requirement_field`]): data on any other op is ordinary
/// payload data. The cognition carriers serve current-authority KE sources
/// only: the `recorded_at` placement is a Memory source view, so it is an
/// unavailable requested view on these carriers — not an empty result
/// (consumer contract §1, data contract §5.2).
pub(crate) fn unsupported_cognition_requirement(
    op: &str,
    payload: &Value,
) -> Option<(&'static str, Map<String, Value>)> {
    let location = cognition_requirement_field(op, payload)?;
    let (value, validation) = if location == "cognition_filter" {
        let value = payload.get("cognition_filter")?;
        (value, validate_cognition_filter(value))
    } else {
        let value = payload.get("scope")?.get("cognition")?;
        (value, validate_cognition_selection(value))
    };
    let unavailable_view = validation.is_ok()
        && location != "cognition_filter"
        && value.get("view").and_then(Value::as_str) == Some("recorded_at");
    let message = if unavailable_view {
        UNAVAILABLE_COGNITION_VIEW_MESSAGE
    } else {
        UNSUPPORTED_COGNITION_MESSAGE
    };
    let field = if unavailable_view {
        format!("{location}.view")
    } else {
        match validation {
            SpokeResult::Ok(()) => return None,
            SpokeResult::Reject(reject) => reject
                .details
                .as_ref()
                .and_then(|details| details.get("field"))
                .and_then(Value::as_str)
                .map_or_else(
                    || location.to_owned(),
                    |relative| {
                        // A non-identifier member already carries its bracket.
                        if relative.starts_with('[') {
                            format!("{location}{relative}")
                        } else {
                            format!("{location}.{relative}")
                        }
                    },
                ),
        }
    };
    let mut details = Map::new();
    details.insert(
        "reason".into(),
        Value::from("unsupported_cognition_requirement"),
    );
    details.insert("field".into(), Value::from(field));
    Some((message, details))
}
