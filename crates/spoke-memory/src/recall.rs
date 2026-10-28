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
    /// Hard item cap.
    pub max_items: usize,
    /// Hard UTF-8 budget over the serialized `content` values of returned
    /// records (§9).
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
pub fn recall<'a>(records: &'a [RecallRecord], request: &RecallRequest<'_>) -> RecallPage<'a> {
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

    let mut page = Vec::new();
    let mut bytes = 0;
    let mut overflow_record_id = None;
    for (_, record) in ranked {
        if page.len() >= request.max_items {
            break;
        }
        let item_bytes = content_wire_bytes(&record.content);
        if let Some(limit) = request.max_bytes {
            if bytes + item_bytes > limit {
                // §9 progress rule: an oversized first item is surfaced whole;
                // later items are deferred rather than silently skipped.
                if page.is_empty() {
                    page.push(record);
                    bytes += item_bytes;
                    overflow_record_id = Some(record.record_id.as_str());
                }
                continue;
            }
        }
        page.push(record);
        bytes += item_bytes;
    }

    RecallPage {
        truncated: page.len() < records.len(),
        records: page,
        total_authorized: records.len(),
        overflow_record_id,
    }
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

/// §9 budget unit: UTF-8 byte length of the **complete serialized `content`
/// value** — field name, quotes, escaping, and `structured` included, e.g.
/// `{"text":"abcd"}` is 15 bytes.
///
/// This deliberately differs from `spoke-operations`' `content_wire_bytes`
/// (text bytes + raw `structured` serialization), which serves the §7.1
/// `max_content_bytes` limit. Issue I-000023 tracks unifying the two
/// measurements; until then the engine measures the wire form on its own
/// because it must not depend on `spoke-operations`.
///
/// `Content` is a `String` plus an optional `serde_json::Value`, so
/// serialization cannot fail.
fn content_wire_bytes(content: &Content) -> usize {
    serde_json::to_vec(content)
        .expect("Content serializes to JSON by construction")
        .len()
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

    #[test]
    fn empty_authorized_set_yields_empty_page() {
        let page = recall(
            &[],
            &RecallRequest {
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
        let page = recall(
            &records,
            &RecallRequest {
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
        let exact = recall(
            &records,
            &RecallRequest {
                query: None,
                max_items: 5,
                max_bytes: Some(30),
            },
        );
        assert_eq!(ids(&exact), vec!["rec_b", "rec_a"]);
        assert!(!exact.truncated);

        let one_byte_short = recall(
            &records,
            &RecallRequest {
                query: None,
                max_items: 5,
                max_bytes: Some(29),
            },
        );
        assert_eq!(ids(&one_byte_short), vec!["rec_b"]);
        assert!(one_byte_short.truncated);
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

        let exact = recall(
            &records,
            &RecallRequest {
                query: None,
                max_items: 5,
                max_bytes: Some(esc_wire + plain_wire),
            },
        );
        assert_eq!(ids(&exact), vec!["rec_esc", "rec_plain"]);
        assert!(!exact.truncated);

        let short = recall(
            &records,
            &RecallRequest {
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
        let page = recall(
            &records,
            &RecallRequest {
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
    fn oversized_first_item_is_surfaced_whole_with_overflow_id() {
        let records = vec![
            record("rec_big", "fact", &"x".repeat(100), "2026-10-02T00:00:00Z"),
            record("rec_small", "note", "tiny", "2026-10-01T00:00:00Z"),
        ];
        let page = recall(
            &records,
            &RecallRequest {
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
        let page = recall(
            &records,
            &RecallRequest {
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
        let page = recall(
            &records,
            &RecallRequest {
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
            query: Some("Where is the GATE key?"),
            max_items: 10,
            max_bytes: None,
        };
        let page = recall(&records, &request);
        let repeated = recall(&records, &request);
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
            query: None,
            max_items: 10,
            max_bytes: None,
        };
        let forward = ids(&recall(&records, &request))
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        records.reverse();
        let reversed = ids(&recall(&records, &request))
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(forward, reversed);
        // rec_c and rec_d tie on kind and timestamp → record_id breaks the tie.
        assert_eq!(forward, vec!["rec_a", "rec_c", "rec_d", "rec_b"]);
    }
}
