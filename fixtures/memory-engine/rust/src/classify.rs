//! Deterministic candidate classification → advisory dispositions (§6.7).

use chrono::{DateTime, Utc};
use spoke_schemas::data::memory_candidate::{Classification, Disposition};

/// Core kinds documented by the protocol (not enforced, §6.6).
const CORE_KINDS: [&str; 3] = ["event", "fact", "note"];
/// Below this UTF-8 byte length a text-only candidate carries too little to retain.
const MIN_TEXT_BYTES: usize = 12;
/// At or above this UTF-8 byte length a candidate is split across records.
const FRAGMENT_TEXT_BYTES: usize = 1024;

/// Pure view of an already-authorized pending candidate (§6.7).
///
/// Providers project their stored candidate onto this shape; the engine never
/// touches storage, clocks, or authorization state.
pub struct CandidateMaterial<'a> {
    /// Proposed kind (open vocabulary, §6.6).
    pub kind: &'a str,
    /// Proposed natural-language content; untrusted data (§10).
    pub text: &'a str,
    /// Whether `proposed.content.structured` is present.
    pub has_structured_content: bool,
    /// Optional event time carried by the candidate (§6.7).
    pub occurred_at: Option<DateTime<Utc>>,
    /// `provenance.source_context_ids` length.
    pub source_context_count: usize,
    /// `provenance.anchors` length.
    pub anchor_count: usize,
}

/// Advisory classification for one candidate (§6.7).
///
/// The reviewer decision remains authoritative; this function never mutates
/// candidate state and never auto-promotes.
///
/// Policy, evaluated in this fixed order:
/// 1. no text and no structured content → `drop` (`empty_content`);
/// 2. text-only candidate below [`MIN_TEXT_BYTES`] → `drop` (`insufficient_text`);
/// 3. text at or above [`FRAGMENT_TEXT_BYTES`] → `fragment` (`oversized_content`,
///    plus `multiple_claims` when four or more sentence runs are present);
/// 4. otherwise → `promote`, annotated with the kind class and the provenance
///    material actually present.
///
/// Confidence is a fixed advisory value per disposition and never depends on
/// time or iteration order.
pub fn classify_candidate(material: &CandidateMaterial<'_>) -> Classification {
    let kind_code = if CORE_KINDS.contains(&material.kind) {
        "core_kind"
    } else {
        "product_kind"
    };

    if material.text.trim().is_empty() && !material.has_structured_content {
        return classified(Disposition::Drop, vec!["empty_content"], 0.0);
    }
    if material.text.len() < MIN_TEXT_BYTES && !material.has_structured_content {
        return classified(Disposition::Drop, vec!["insufficient_text", kind_code], 0.2);
    }
    if material.text.len() >= FRAGMENT_TEXT_BYTES {
        let mut codes = vec!["oversized_content"];
        if sentence_count(material.text) >= 4 {
            codes.push("multiple_claims");
        }
        codes.push(kind_code);
        return classified(Disposition::Fragment, codes, 0.6);
    }

    let mut codes = vec![kind_code];
    if material.anchor_count > 0 {
        codes.push("sourced_anchor");
    }
    if material.source_context_count > 0 {
        codes.push("provenance_context");
    }
    if material.occurred_at.is_some() {
        codes.push("occurred_at_known");
    }
    classified(Disposition::Promote, codes, 0.8)
}

fn classified(
    disposition: Disposition,
    rationale_codes: Vec<&str>,
    confidence: f64,
) -> Classification {
    Classification {
        confidence: Some(confidence),
        disposition,
        rationale_codes: rationale_codes.into_iter().map(str::to_string).collect(),
    }
}

/// Count non-empty runs separated by `.`, `!`, `?`, or a newline.
fn sentence_count(text: &str) -> usize {
    text.split(['.', '!', '?', '\n'])
        .filter(|run| !run.trim().is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn material<'a>(kind: &'a str, text: &'a str) -> CandidateMaterial<'a> {
        CandidateMaterial {
            kind,
            text,
            has_structured_content: false,
            occurred_at: None,
            source_context_count: 0,
            anchor_count: 0,
        }
    }

    fn wire(classification: &Classification) -> serde_json::Value {
        serde_json::to_value(classification).expect("classification serializes")
    }

    #[test]
    fn empty_content_is_dropped() {
        let classification = classify_candidate(&material("note", "   \n\t "));
        assert_eq!(
            wire(&classification),
            json!({
                "disposition": "drop",
                "rationale_codes": ["empty_content"],
                "confidence": 0.0,
            })
        );
    }

    #[test]
    fn short_text_without_structured_content_is_dropped() {
        let classification = classify_candidate(&material("event", "key"));
        assert_eq!(
            wire(&classification),
            json!({
                "disposition": "drop",
                "rationale_codes": ["insufficient_text", "core_kind"],
                "confidence": 0.2,
            })
        );
    }

    #[test]
    fn short_text_with_structured_content_is_promoted() {
        // Structured content carries the claim even when the prose is terse.
        let mut input = material("event", "key");
        input.has_structured_content = true;
        let classification = classify_candidate(&input);
        assert_eq!(classification.disposition, Disposition::Promote);
        assert!(!classification
            .rationale_codes
            .iter()
            .any(|code| code == "insufficient_text"));
    }

    #[test]
    fn oversized_single_sentence_is_fragmented_without_multi_claim() {
        let text = "a long single claim ".repeat(60);
        let classification = classify_candidate(&material("fact", &text));
        assert_eq!(classification.disposition, Disposition::Fragment);
        assert_eq!(
            classification.rationale_codes,
            vec!["oversized_content".to_string(), "core_kind".to_string()]
        );
        assert_eq!(classification.confidence, Some(0.6));
    }

    #[test]
    fn oversized_multi_sentence_carries_multiple_claims() {
        let text = format!(
            "{}. {}. {}. {}{}",
            "one".repeat(80),
            "two".repeat(80),
            "three".repeat(80),
            "four".repeat(80),
            "."
        );
        let classification = classify_candidate(&material("note", &text));
        assert_eq!(classification.disposition, Disposition::Fragment);
        assert_eq!(
            classification.rationale_codes,
            vec![
                "oversized_content".to_string(),
                "multiple_claims".to_string(),
                "core_kind".to_string(),
            ]
        );
    }

    #[test]
    fn durable_statement_is_promoted_with_provenance_rationale() {
        let mut input = material("fact", "Mira promised to return before dawn.");
        input.occurred_at = Some("2026-10-28T09:15:00Z".parse().expect("timestamp parses"));
        input.source_context_count = 2;
        input.anchor_count = 1;
        let classification = classify_candidate(&input);
        assert_eq!(
            wire(&classification),
            json!({
                "disposition": "promote",
                "rationale_codes": [
                    "core_kind",
                    "sourced_anchor",
                    "provenance_context",
                    "occurred_at_known",
                ],
                "confidence": 0.8,
            })
        );
    }

    #[test]
    fn unknown_kind_is_reported_as_product_kind() {
        let classification = classify_candidate(&material(
            "character_trait",
            "Mira distrusts the harbor guild.",
        ));
        assert_eq!(classification.disposition, Disposition::Promote);
        assert_eq!(
            classification.rationale_codes,
            vec!["product_kind".to_string()]
        );
    }

    #[test]
    fn classification_is_deterministic() {
        let input = material(
            "note",
            "The gate key hangs behind the harbor master's desk.",
        );
        let first = wire(&classify_candidate(&input));
        let second = wire(&classify_candidate(&input));
        assert_eq!(first, second);
    }
}
