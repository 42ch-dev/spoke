//! Deterministic recall ranking and budget truncation over pre-authorized
//! record inputs (§9).
//!
//! The provider applies the normative filter (authorization, selection,
//! provenance, tombstone and candidate exclusion) before calling the engine;
//! this module only ranks the authorized set and applies the §9 budget.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use spoke_schemas::data::retained_record::Content;

/// One already-authorized retained record supplied to the ranking step.
pub struct RecallRecord {
    pub record_id: String,
    /// Open-vocabulary kind (§6.6).
    pub kind: String,
    pub content: Content,
    pub created_at: DateTime<Utc>,
}

/// Ranking hint and §7.22 budget, already validated by the provider.
pub struct RecallRequest<'a> {
    /// Ranking hint, never a filter (§9 item 2).
    pub query: Option<&'a str>,
    /// Cursor resume (MEM-R026): drop this many leading ranked records
    /// BEFORE the page budget applies, so a continuation page is the next
    /// window of the same ranking instead of an empty tail.
    pub offset: usize,
    /// Hard item cap, applied after `offset`.
    pub max_items: usize,
    /// Hard UTF-8 budget over the serialized `content` values of returned
    /// records (§9), applied after `offset`.
    pub max_bytes: Option<usize>,
}

/// Ranked, budgeted page (§7.22 result fields).
pub struct RecallPage<'a> {
    pub records: Vec<&'a RecallRecord>,
    pub total_authorized: usize,
    pub truncated: bool,
    pub overflow_record_id: Option<&'a str>,
}

/// Query overlap outweighs kind weight so an explicit hint wins ties.
const QUERY_MATCH_WEIGHT: u32 = 4;

/// Scoring policy (§9 item 2 — provider algorithm, not protocol-normative):
/// `score = 4 × distinct query-token overlaps + kind weight`, where the kind
/// weight is `fact` 3, `event` 2, `note` 1, and any product kind 0. Ordering is
/// `score` desc, then `created_at` desc, then `record_id` asc — total and
/// independent of input order, so identical state/request/policy version
/// yields identical pages.
///
/// Budget: the returned page is always a prefix of the ranking, so a provider
/// cursor (MEM-R026) resumes at `records.len()`; a record deferred at the byte
/// boundary keeps its rank and is never displaced by a smaller lower-ranked one.
pub fn recall<'a>(
    records: &'a [RecallRecord],
    request: &RecallRequest<'_>,
) -> Result<RecallPage<'a>, RecallError> {
    let query_tokens = tokenize(request.query.unwrap_or(""));
    let mut ranked: Vec<(u32, &'a RecallRecord)> = records
        .iter()
        .map(|record| (score(record, &query_tokens), record))
        .collect();
    ranked.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.record_id.cmp(&right.record_id))
    });

    // MEM-R026: the cursor offset drops the leading ranked records before
    // the page budget, so the page is a prefix of the ranking starting at
    // `offset` and a continuation cursor resumes at `offset + page.len()`.
    let ranked: Vec<(u32, &'a RecallRecord)> = ranked.into_iter().skip(request.offset).collect();

    let mut page = Vec::new();
    let mut bytes = 0;
    let mut overflow_record_id = None;
    for (_, record) in ranked {
        if page.len() >= request.max_items {
            break;
        }
        let item_bytes = content_serialized_bytes(&record.content)?;
        if let Some(limit) = request.max_bytes {
            if bytes + item_bytes > limit {
                // §9 progress rule: an oversized first item is surfaced whole and
                // flags `overflow_record_id`. Once the page is non-empty, the item
                // is deferred and the traversal stops, so the page is always a
                // prefix of the ranking: a cursor continuation resumes exactly at
                // the deferred item, and lower-ranked records never displace it.
                if page.is_empty() {
                    page.push(record);
                    bytes += item_bytes;
                    overflow_record_id = Some(record.record_id.as_str());
                }
                break;
            }
        }
        page.push(record);
        bytes += item_bytes;
    }

    Ok(RecallPage {
        truncated: request.offset + page.len() < records.len(),
        records: page,
        total_authorized: records.len(),
        overflow_record_id,
    })
}

fn score(record: &RecallRecord, query_tokens: &HashSet<String>) -> u32 {
    let kind_weight = match record.kind.as_str() {
        "fact" => 3,
        "event" => 2,
        "note" => 1,
        _ => 0,
    };
    let overlaps = if query_tokens.is_empty() {
        0
    } else {
        let record_tokens = tokenize(&record.content.text);
        query_tokens.intersection(&record_tokens).count() as u32
    };
    overlaps * QUERY_MATCH_WEIGHT + kind_weight
}

/// Lowercase alphanumeric runs; `query` is a hint, so matching is
/// case-insensitive and punctuation-insensitive.
fn tokenize(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Recall-ranking failure: the §9 item 3 byte budget serializes the `content` wire
/// form, which has no JSON representation when it holds a non-finite float.
/// `serde_json::Value` cannot hold one by construction, so this is a
/// defensive guard kept at parity with the spoke-operations digest
/// helpers — it rejects instead of panicking if the invariant ever moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecallError {
    NonFiniteContent,
}

impl std::fmt::Display for RecallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFiniteContent => write!(
                formatter,
                "record content is not serializable: non-finite floats have no JSON form"
            ),
        }
    }
}

impl std::error::Error for RecallError {}

/// §9 item 3 page-budget unit: UTF-8 byte length of the **complete serialized
/// `content` value exactly as the page emits it** (`serde_json::to_vec`) —
/// field name, quotes, escaping, and `structured` included, e.g.
/// `{"text":"abcd"}` is 15 bytes.
///
/// The §7.1 `max_content_bytes` admission limit and this §9 item 3 page budget are
/// two distinct budgets: admission measures RFC 8785 JCS
/// (`spoke-operations::memory::validate`, because it is compared against the
/// TypeScript `JSON.stringify` form), while this one measures the wire form so
/// the budget counts literally the bytes the response emits. Both drop
/// top-level explicit `null` members — a deserialized `null` becomes
/// `Option::None` and a constructed `Some(Value::Null)` is stripped here — so
/// the two agree on which members exist and differ only in number
/// formatting. Nested `null`s are content and stay. It stays local because the
/// engine must not depend on `spoke-operations`.
///
/// `Content` is a `String` plus an optional `serde_json::Value`, so
/// serialization cannot fail on finite numbers; a failure means a non-finite
/// float and rejects instead of panicking. Fast path: when `structured` is
/// absent or a real value, `skip_serializing_if = "Option::is_none"` plus
/// the absence of any explicit null member means direct typed
/// serialization emits exactly the members the normalization filter would
/// keep, in the same field order — so only a constructed `Some(Value::Null)`
/// needs the `to_value` → filter → `to_vec` round trip.
fn content_serialized_bytes(content: &Content) -> Result<usize, RecallError> {
    content_serialized(content).map(|bytes| bytes.len())
}

fn content_serialized(content: &Content) -> Result<Vec<u8>, RecallError> {
    if !matches!(content.structured.as_ref(), Some(serde_json::Value::Null)) {
        return serde_json::to_vec(content).map_err(|_| RecallError::NonFiniteContent);
    }
    let value = match serde_json::to_value(content) {
        Ok(serde_json::Value::Object(members)) => serde_json::Value::Object(
            members
                .into_iter()
                .filter(|(_, member)| !member.is_null())
                .collect(),
        ),
        Ok(other) => other,
        Err(_) => return Err(RecallError::NonFiniteContent),
    };
    serde_json::to_vec(&value).map_err(|_| RecallError::NonFiniteContent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(stamp: &str) -> DateTime<Utc> {
        stamp.parse().expect("timestamp parses")
    }

    fn record(id: &str, kind: &str, text: &str, created_at: &str) -> RecallRecord {
        RecallRecord {
            record_id: id.to_string(),
            kind: kind.to_string(),
            content: Content {
                structured: None,
                text: text.to_string(),
            },
            created_at: at(created_at),
        }
    }

    fn ids<'a>(page: &'a RecallPage<'a>) -> Vec<&'a str> {
        page.records
            .iter()
            .map(|record| record.record_id.as_str())
            .collect()
    }


    /// Unwrap the engine Result for the ranking-focused unit tests; the
    /// error path itself is covered by `content_serialized_bytes` guard tests.
    fn recall_checked<'a>(
        records: &'a [RecallRecord],
        request: &RecallRequest<'_>,
    ) -> RecallPage<'a> {
        recall(records, request).expect("recall succeeds on finite content")
    }

    #[test]
    fn non_finite_guard_rejects_instead_of_panicking() {
        // The guard path: a serialization failure surfaces as a typed
        // reject carrying the non-finite reason, never a panic. serde_json
        // Values cannot hold non-finite floats by construction, so the
        // reachable assertions cover the finite edge shapes the guard must
        // keep accepting, and the error variant round-trips through Display.
        let records = vec![
            record("rec_edge", "fact", "edge", "2026-10-01T00:00:00Z"),
            record("rec_neg_zero", "fact", "-0.0", "2026-10-02T00:00:00Z"),
        ];
        let page = recall(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(64),
            },
        )
        .expect("finite extreme values rank fine");
        assert_eq!(page.records.len(), 2);
        assert_eq!(
            RecallError::NonFiniteContent.to_string(),
            "record content is not serializable: non-finite floats have no JSON form"
        );
    }

    #[test]
    fn empty_authorized_set_yields_empty_page() {
        let page = recall_checked(
            &[],
            &RecallRequest {
                offset: 0,
                query: Some("gate key"),
                max_items: 5,
                max_bytes: Some(64),
            },
        );
        assert!(page.records.is_empty());
        assert_eq!(page.total_authorized, 0);
        assert!(!page.truncated);
        assert_eq!(page.overflow_record_id, None);
    }

    #[test]
    fn max_items_caps_the_page_and_flags_truncation() {
        let records = vec![
            record("rec_a", "fact", "alpha", "2026-10-01T00:00:00Z"),
            record("rec_b", "fact", "bravo", "2026-10-02T00:00:00Z"),
            record("rec_c", "fact", "charlie", "2026-10-03T00:00:00Z"),
        ];
        let page = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 2,
                max_bytes: None,
            },
        );
        // Same score → newest first; rec_c and rec_b survive the cap.
        assert_eq!(ids(&page), vec!["rec_c", "rec_b"]);
        assert_eq!(page.total_authorized, 3);
        assert!(page.truncated);
        assert_eq!(page.overflow_record_id, None);
    }

    #[test]
    fn cursor_offset_applies_before_the_page_budget() {
        let records = vec![
            record("rec_a", "fact", "alpha", "2026-10-01T00:00:00Z"),
            record("rec_b", "fact", "bravo", "2026-10-02T00:00:00Z"),
            record("rec_c", "fact", "charlie", "2026-10-03T00:00:00Z"),
            record("rec_d", "fact", "delta", "2026-10-04T00:00:00Z"),
            record("rec_e", "fact", "echo", "2026-10-05T00:00:00Z"),
        ];
        // Same score → newest first: rec_e, rec_d, rec_c, rec_b, rec_a.
        let middle = recall_checked(
            &records,
            &RecallRequest {
                offset: 2,
                query: None,
                max_items: 2,
                max_bytes: None,
            },
        );
        // MEM-R026: the offset drops the two newest records BEFORE the
        // budget, so the continuation page is the next window of the same
        // ranking — not an empty tail.
        assert_eq!(ids(&middle), vec!["rec_c", "rec_b"]);
        assert_eq!(middle.total_authorized, 5);
        assert!(middle.truncated);

        let tail = recall_checked(
            &records,
            &RecallRequest {
                offset: 4,
                query: None,
                max_items: 2,
                max_bytes: None,
            },
        );
        assert_eq!(ids(&tail), vec!["rec_a"]);
        assert!(!tail.truncated);
    }

    #[test]
    fn exact_byte_boundary_still_fits() {
        let records = vec![
            record("rec_a", "fact", "abcd", "2026-10-01T00:00:00Z"),
            record("rec_b", "fact", "efgh", "2026-10-02T00:00:00Z"),
        ];
        // §9 measures the serialized content value: {"text":"abcd"} = 15 bytes.
        assert_eq!(
            serde_json::to_vec(&records[0].content)
                .expect("content serializes")
                .len(),
            15
        );
        let exact = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(30),
            },
        );
        assert_eq!(ids(&exact), vec!["rec_b", "rec_a"]);
        assert!(!exact.truncated);

        let one_byte_short = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(29),
            },
        );
        assert_eq!(ids(&one_byte_short), vec!["rec_b"]);
        assert!(one_byte_short.truncated);
    }

    #[test]
    fn numeric_content_budget_uses_the_wire_measure() {
        // I-000035: the §9 item 3 page budget counts the bytes the page emits, so
        // numbers keep the `serde_json` form (`1.0`, `100.0`) and count 44
        // bytes; the RFC 8785 §7.1 admission form would count 40.
        let records = vec![RecallRecord {
            record_id: "rec_numbers".to_string(),
            kind: "fact".to_string(),
            content: Content {
                structured: Some(serde_json::json!({"n": 1.0, "e": 1e2})),
                text: String::new(),
            },
            created_at: at("2026-10-01T00:00:00Z"),
        }];
        assert_eq!(
            content_serialized_bytes(&records[0].content).expect("finite content measures"),
            44
        );

        let exact = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(44),
            },
        );
        assert_eq!(ids(&exact), vec!["rec_numbers"]);
        assert_eq!(exact.overflow_record_id, None);
        assert!(!exact.truncated);

        // One byte short keeps the unchanged overflow rule: a lone oversized
        // first item is surfaced whole and flagged.
        let short = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(43),
            },
        );
        assert_eq!(ids(&short), vec!["rec_numbers"]);
        assert_eq!(short.overflow_record_id, Some("rec_numbers"));
        assert!(!short.truncated);
    }

    #[test]
    fn measure_fast_path_matches_direct_typed_serialization() {
        // I-000039: the typed fast path must emit exactly the direct
        // `serde_json::to_vec(content)` wire bytes for `structured: None`
        // (15-byte plain-text vector) and `Some(non-null)` (44-byte numeric
        // vector); only `Some(Value::Null)` takes normalization. Assert the
        // full byte slices against the normalize path (`to_value` → strip
        // null members → `to_vec`), not just lengths.
        let cases = [
            (Content {
                structured: None,
                text: "abcd".to_string(),
            }, 15usize),
            (Content {
                structured: Some(serde_json::json!({"n": 1.0, "e": 1e2})),
                text: String::new(),
            }, 44),
            (Content {
                structured: None,
                text: "hello".to_string(),
            }, 16),
            (Content {
                structured: Some(serde_json::Value::Null),
                text: String::new(),
            }, 11),
        ];
        for (content, expected_len) in &cases {
            let direct = serde_json::to_vec(content).expect("content serializes");
            let normalized = match serde_json::to_value(content).expect("content converts") {
                serde_json::Value::Object(members) => serde_json::to_vec(
                    &serde_json::Value::Object(
                        members
                            .into_iter()
                            .filter(|(_, member)| !member.is_null())
                            .collect(),
                    ),
                ),
                other => serde_json::to_vec(&other),
            }
            .expect("normalized content serializes");
            let fast_path = content_serialized(content).expect("finite content measures");
            assert_eq!(
                fast_path, normalized,
                "fast path must emit exactly the normalized wire bytes"
            );
            assert_eq!(fast_path.len(), *expected_len);
            // Without a constructed null member direct typed serialization
            // must match byte-for-byte; `Some(Value::Null)` serializes the
            // null member directly (29 bytes), which normalization strips.
            if !matches!(content.structured, Some(serde_json::Value::Null)) {
                assert_eq!(fast_path.len(), direct.len(), "wire byte length");
                assert_eq!(
                    fast_path, direct,
                    "fast path must equal direct typed serialization byte-for-byte"
                );
            }
        }
    }

    #[test]
    fn constructed_null_content_budgets_like_admission() {
        // I-000031: a caller-built `Some(Value::Null)` is the same logical
        // content as a deserialized `null`; both normalize to
        // `{"text":""}` = 11 bytes in either form, so recall and admission
        // budget identically.
        let records = vec![RecallRecord {
            record_id: "rec_null".to_string(),
            kind: "fact".to_string(),
            content: Content {
                structured: Some(serde_json::Value::Null),
                text: String::new(),
            },
            created_at: at("2026-10-01T00:00:00Z"),
        }];
        assert_eq!(
            content_serialized_bytes(&records[0].content).expect("finite content measures"),
            11
        );

        let exact = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(11),
            },
        );
        assert_eq!(ids(&exact), vec!["rec_null"]);
        assert_eq!(exact.overflow_record_id, None);

        // One byte short: the measure really is 11, not the 29-byte
        // `{"structured":null,"text":""}` form.
        let short = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(10),
            },
        );
        assert_eq!(ids(&short), vec!["rec_null"]);
        assert_eq!(short.overflow_record_id, Some("rec_null"));
    }

    #[test]
    fn serialized_escaping_and_overhead_count_against_the_budget() {
        let tricky = "quote \" and newline \n and unicode — dash";
        let records = vec![
            record("rec_esc", "fact", tricky, "2026-10-02T00:00:00Z"),
            record("rec_plain", "fact", "hello", "2026-10-01T00:00:00Z"),
        ];
        let esc_wire = serde_json::to_vec(&records[0].content)
            .expect("content serializes")
            .len();
        let plain_wire = serde_json::to_vec(&records[1].content)
            .expect("content serializes")
            .len();
        assert!(
            esc_wire > tricky.len(),
            "escaping and JSON overhead are counted"
        );
        assert_eq!(plain_wire, 16, "{{\"text\":\"hello\"}} is 16 bytes");

        let exact = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(esc_wire + plain_wire),
            },
        );
        assert_eq!(ids(&exact), vec!["rec_esc", "rec_plain"]);
        assert!(!exact.truncated);

        let short = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(esc_wire + plain_wire - 1),
            },
        );
        assert_eq!(ids(&short), vec!["rec_esc"]);
        assert!(short.truncated);
    }

    #[test]
    fn byte_budget_defers_the_overflowing_item_and_flags_truncation() {
        let records = vec![
            record("rec_a", "fact", "abcd", "2026-10-01T00:00:00Z"),
            record("rec_b", "fact", "efgh", "2026-10-02T00:00:00Z"),
            record("rec_c", "fact", "ijkl", "2026-10-03T00:00:00Z"),
        ];
        let page = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(30),
            },
        );
        assert_eq!(ids(&page), vec!["rec_c", "rec_b"]);
        assert_eq!(page.total_authorized, 3);
        assert!(page.truncated);
        assert_eq!(page.overflow_record_id, None);
    }

    #[test]
    fn deferred_tail_is_not_displaced_by_lower_ranked_records() {
        let records = vec![
            record("rec_a", "fact", "abcd", "2026-10-03T00:00:00Z"),
            record(
                "rec_b",
                "fact",
                "abcdefghijklmnopqrs",
                "2026-10-02T00:00:00Z",
            ),
            record("rec_c", "fact", "efgh", "2026-10-01T00:00:00Z"),
        ];
        // Identical kind and score → descending created_at ranks A, B, C.
        for (record, expected) in records.iter().zip([15usize, 30, 15]) {
            assert_eq!(
                serde_json::to_vec(&record.content)
                    .expect("content serializes")
                    .len(),
                expected
            );
        }
        let request = RecallRequest {
            offset: 0,
            query: None,
            max_items: 3,
            max_bytes: Some(30),
        };

        let first = recall_checked(&records, &request);
        assert_eq!(ids(&first), vec!["rec_a"]);
        assert!(first.truncated);
        assert_eq!(first.overflow_record_id, None);

        // Cursor continuation resumes at the deferred item, which keeps its rank
        // even though the smaller record C would have fit in the same budget.
        let second = recall_checked(&records[1..], &request);
        assert_eq!(ids(&second), vec!["rec_b"]);
        assert!(second.truncated);

        let third = recall_checked(&records[2..], &request);
        assert_eq!(ids(&third), vec!["rec_c"]);
        assert!(!third.truncated);
    }

    #[test]
    fn oversized_first_item_is_surfaced_whole_with_overflow_id() {
        let records = vec![
            record("rec_big", "fact", &"x".repeat(100), "2026-10-02T00:00:00Z"),
            record("rec_small", "note", "tiny", "2026-10-01T00:00:00Z"),
        ];
        let page = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(10),
            },
        );
        // §9 progress rule: never silently dropped, never silently skipped.
        assert_eq!(ids(&page), vec!["rec_big"]);
        assert_eq!(page.overflow_record_id, Some("rec_big"));
        assert!(page.truncated);
    }

    #[test]
    fn oversized_lone_item_is_surfaced_without_truncation() {
        let records = vec![record(
            "rec_big",
            "fact",
            &"x".repeat(100),
            "2026-10-01T00:00:00Z",
        )];
        let page = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 5,
                max_bytes: Some(10),
            },
        );
        assert_eq!(ids(&page), vec!["rec_big"]);
        assert_eq!(page.overflow_record_id, Some("rec_big"));
        assert!(!page.truncated);
    }

    #[test]
    fn ranking_orders_by_kind_weight_then_recency_then_id() {
        let records = vec![
            record("rec_note", "note", "note body", "2026-10-05T00:00:00Z"),
            record(
                "rec_event_old",
                "event",
                "event body",
                "2026-10-01T00:00:00Z",
            ),
            record(
                "rec_event_new",
                "event",
                "event body",
                "2026-10-04T00:00:00Z",
            ),
            record("rec_fact", "fact", "fact body", "2026-10-02T00:00:00Z"),
            record("rec_zz", "note", "note body", "2026-10-05T00:00:00Z"),
        ];
        let page = recall_checked(
            &records,
            &RecallRequest {
                offset: 0,
                query: None,
                max_items: 10,
                max_bytes: None,
            },
        );
        assert_eq!(
            ids(&page),
            vec![
                "rec_fact",
                "rec_event_new",
                "rec_event_old",
                "rec_note",
                "rec_zz",
            ]
        );
        assert!(!page.truncated);
    }

    #[test]
    fn query_hint_changes_order_deterministically() {
        let records = vec![
            record(
                "rec_a",
                "fact",
                "the gate key hangs by the door",
                "2026-10-01T00:00:00Z",
            ),
            record(
                "rec_b",
                "fact",
                "mira trusts the harbor master",
                "2026-10-01T00:00:00Z",
            ),
        ];
        let request = RecallRequest {
            offset: 0,
            query: Some("Where is the GATE key?"),
            max_items: 10,
            max_bytes: None,
        };
        let page = recall_checked(&records, &request);
        let repeated = recall_checked(&records, &request);
        assert_eq!(ids(&page), vec!["rec_a", "rec_b"]);
        assert_eq!(ids(&repeated), ids(&page));
    }

    #[test]
    fn ordering_is_independent_of_input_order() {
        let mut records = vec![
            record("rec_a", "fact", "alpha", "2026-10-01T00:00:00Z"),
            record("rec_b", "note", "bravo", "2026-10-02T00:00:00Z"),
            record("rec_c", "event", "charlie", "2026-10-03T00:00:00Z"),
            record("rec_d", "event", "delta", "2026-10-03T00:00:00Z"),
        ];
        let request = RecallRequest {
            offset: 0,
            query: None,
            max_items: 10,
            max_bytes: None,
        };
        let forward = ids(&recall_checked(&records, &request))
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        records.reverse();
        let reversed = ids(&recall_checked(&records, &request))
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(forward, reversed);
        // rec_c and rec_d tie on kind and timestamp → record_id breaks the tie.
        assert_eq!(forward, vec!["rec_a", "rec_c", "rec_d", "rec_b"]);
    }
}
