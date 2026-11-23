//! `spoke-knowledge-adoption` family helpers: the `adoption_anchor`
//! immutability gate for the normal put path and the bounded
//! retirement-chain resolver (spoke-knowledge-adoption contract §Anchor
//! immutability / §Retirement-chain resolution).
//!
//! Anchor equality is RFC 8785 canonical-JSON byte equality — the same
//! rule [`crate::revision_snapshots_equal_canonical`] applies — so a put
//! that carries the identical stored anchor never refuses.
//!
//! TypeScript parity: `packages/spoke-operations/src/adoption.ts`.

use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde::Serialize;
use serde_json::{json, Map, Value};
use spoke_schemas::KnowledgeEntry;
use std::collections::HashSet;
use std::future::Future;

/// Retirement edges followed after the requested entry before `SOURCE_RETIRED`.
pub const RETIREMENT_CHAIN_MAX_EDGES: usize = 32;

/// Legacy product-private adoption encoding namespace (the superseded
/// `source_anchor` + `extensions.mindcore_adoption` codec the protocol
/// `adoption_anchor` field replaces). Destination copies drop this
/// namespace; every other extension namespace round-trips unchanged.
pub const ADOPTION_LEGACY_EXTENSIONS_NAMESPACE: &str = "mindcore_adoption";

/// RFC 8785 canonical JSON bytes of `value`, or `None` when the value
/// cannot be canonicalized (mirrors the TypeScript `canonicalize` failure
/// branch). Callers must refuse on `None`; it is not a comparable byte
/// value — two failures do not mean equality.
fn canonical_bytes<T: Serialize>(value: &T) -> Option<Vec<u8>> {
    serde_jcs::to_vec(value).ok()
}

/// The `adoption_anchor` immutability gate for a normal put
/// (spoke-knowledge-adoption contract §Anchor immutability): once the
/// stored entry carries an `adoption_anchor`, a put must preserve it
/// unchanged. A candidate that replaces it with a different value or
/// removes it refuses `ADOPTION_ANCHOR_IMMUTABLE` with `details.entry_id`
/// naming the destination entry; a candidate carrying the identical anchor
/// (canonical-byte equal) or no anchor when the stored entry has none
/// passes.
pub fn assert_adoption_anchor_preserved(
    candidate: &KnowledgeEntry,
    stored: Option<&KnowledgeEntry>,
) -> SpokeResult<()> {
    let Some(stored_anchor) = stored.and_then(|stored| stored.adoption_anchor.as_ref()) else {
        return spoke_ok_unit();
    };
    // Both canonical strings are computed before comparing: a failed
    // canonicalization means "cannot prove unchanged" and refuses
    // `INVALID_INPUT` — two failures must not compare equal.
    let verdict = match candidate.adoption_anchor.as_ref() {
        Some(candidate_anchor) => anchor_verdict(
            canonical_bytes(candidate_anchor),
            canonical_bytes(stored_anchor),
        ),
        None => AnchorVerdict::Immutable,
    };
    match verdict {
        AnchorVerdict::Preserved => spoke_ok_unit(),
        AnchorVerdict::InvalidInput => spoke_reject(
            SpokeRejectCode::InvalidInput,
            "JCS canonicalization failed: adoption_anchor is not JSON-canonicalizable (RFC 8785)",
            None,
        ),
        AnchorVerdict::Immutable => anchor_immutable_reject(candidate, stored),
    }
}

/// The gate's verdict once the stored entry is known to carry an anchor.
enum AnchorVerdict {
    /// The candidate anchor canonicalizes to the stored anchor's bytes.
    Preserved,
    /// Either anchor failed canonicalization: refuse `INVALID_INPUT`.
    /// `None` is "cannot prove unchanged", never byte equality.
    InvalidInput,
    /// The anchor was removed or canonically differs: refuse
    /// `ADOPTION_ANCHOR_IMMUTABLE`.
    Immutable,
}

/// Byte-equality half of the gate: only `Some == Some` proves preservation.
fn anchor_verdict(
    candidate_bytes: Option<Vec<u8>>,
    stored_bytes: Option<Vec<u8>>,
) -> AnchorVerdict {
    match (candidate_bytes, stored_bytes) {
        (Some(candidate_bytes), Some(stored_bytes)) => {
            if candidate_bytes == stored_bytes {
                AnchorVerdict::Preserved
            } else {
                AnchorVerdict::Immutable
            }
        }
        _ => AnchorVerdict::InvalidInput,
    }
}

/// `ADOPTION_ANCHOR_IMMUTABLE` refusal with `details.entry_id` naming the
/// destination entry.
fn anchor_immutable_reject(
    candidate: &KnowledgeEntry,
    stored: Option<&KnowledgeEntry>,
) -> SpokeResult<()> {
    let mut details = Map::new();
    details.insert(
        "entry_id".into(),
        Value::String(
            stored
                .map(|stored| stored.entry_id.clone())
                .unwrap_or_else(|| candidate.entry_id.clone()),
        ),
    );
    spoke_reject(
        SpokeRejectCode::AdoptionAnchorImmutable,
        "a normal put must preserve the stored adoption_anchor unchanged",
        Some(details),
    )
}

/// One visited retirement-chain node: the entry id plus the terminal-entry
/// fields the resolver needs (`status`, and `scope_id` for the recorded
/// terminal scope). Successor traversal is delegated to the provider —
/// how a `merged` entry names its successors is storage-owned; the helper
/// owns only the chain's termination semantics.
#[derive(Debug, Clone)]
pub struct RetirementChainEntry {
    /// Entry id as visited (the requested id first).
    pub entry_id: String,
    /// Scope id the entry was stored under.
    pub scope_id: String,
    /// Status vocabulary (`merged` follows edges, `deleted` is terminal refusal).
    pub status: String,
    /// Revision of this entry (recorded on the anchor for the terminal).
    pub revision: Option<u64>,
}

/// Successful resolution: the terminal live source the anchor records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetirementTerminal {
    /// Resolved terminal source scope id.
    pub source_scope_id: String,
    /// Resolved terminal source entry id.
    pub source_entry_id: String,
    /// Resolved terminal source revision (the node's own `revision`, or 0).
    pub source_revision: u64,
}

/// Resolve a requested source's retirement chain to the terminal live
/// entry (spoke-knowledge-adoption contract §Retirement-chain resolution).
///
/// `requested` is the already-loaded, already-admitted requested entry (a
/// missing or non-disclosable source refuses `NOT_FOUND` before this
/// helper runs — the resolver never sees it). A live requested source
/// resolves to itself without touching the chain. From a `merged` node
/// resolution follows at most [`RETIREMENT_CHAIN_MAX_EDGES`] (32) edges;
/// every visited entry id is tracked, and `details.retirement_chain` on
/// `SOURCE_RETIRED` is the ordered visited-id array beginning with the
/// requested id and ending at the last resolved node — the entry at
/// which resolution failed. Refusals: a repeated visited id (cycle),
/// zero or multiple successors on a `merged` node (ambiguous chain), a
/// `deleted` terminal, a mid-chain load miss, or a required 33rd edge
/// (the successor beyond the rejected edge is never listed).
///
/// `successors_of` is the storage-owned successor lookup: `None` means the
/// provider could not identify successors (treated as ambiguous), an empty
/// list means no successor, more than one means an ambiguous merge.
/// `load_entry` loads the node stored under an id in the source scope;
/// a mid-chain `None` refuses `SOURCE_RETIRED` (the requested entry's own
/// absence is caller-owned `NOT_FOUND` before the resolver runs).
pub async fn resolve_retirement_chain<S, SFut, L, LFut>(
    requested: RetirementChainEntry,
    mut successors_of: S,
    load_entry: L,
) -> SpokeResult<RetirementTerminal>
where
    S: FnMut(&RetirementChainEntry) -> SFut,
    SFut: Future<Output = Option<Vec<String>>>,
    L: Fn(&str) -> LFut,
    LFut: Future<Output = Option<RetirementChainEntry>>,
{
    let mut visited: Vec<String> = vec![requested.entry_id.clone()];
    let mut seen: HashSet<String> = visited.iter().cloned().collect();
    let mut current = requested;
    while current.status == "merged" {
        let successors = successors_of(&current).await;
        let Some(successors) = successors else {
            return source_retired(
                "retirement chain is ambiguous: a merged entry has zero or multiple successors",
                &visited,
            );
        };
        if successors.len() != 1 {
            return source_retired(
                "retirement chain is ambiguous: a merged entry has zero or multiple successors",
                &visited,
            );
        }
        let successor_id = successors.into_iter().next().expect("exactly one");
        if visited.len() > RETIREMENT_CHAIN_MAX_EDGES {
            // Requiring a 33rd edge means 32 edges were already followed
            // and the chain still has not terminated: the refusal trace
            // ends at the last resolved (still-`merged`) node — the
            // successor beyond the rejected edge stays unlisted
            // (contract §Retirement-chain resolution).
            return source_retired("retirement chain exceeds the 32-edge limit", &visited);
        }
        if seen.contains(&successor_id) {
            visited.push(successor_id);
            return source_retired("retirement chain cycle: a visited entry id repeats", &visited);
        }
        // The successor must resolve before it is listed: a missing or
        // unresolvable target refuses with the chain ending at the last
        // resolved node — the unresolvable target stays unlisted
        // (contract §Retirement-chain resolution).
        let Some(successor) = load_entry(&successor_id).await else {
            return source_retired(
                "retirement chain is broken: the successor entry is missing",
                &visited,
            );
        };
        visited.push(successor_id.clone());
        seen.insert(successor_id);
        current = successor;
    }
    if current.status == "deleted" {
        return source_retired("retirement chain terminal entry is deleted", &visited);
    }
    spoke_ok(RetirementTerminal {
        source_scope_id: current.scope_id,
        source_entry_id: current.entry_id,
        source_revision: current.revision.unwrap_or(0),
    })
}

/// `SOURCE_RETIRED` with `details.retirement_chain` — the ordered visited
/// entry ids, requested first and including the refusal node.
fn source_retired<T>(message: &str, visited: &[String]) -> SpokeResult<T> {
    let mut details = Map::new();
    details.insert(
        "retirement_chain".into(),
        json!(visited.iter().map(String::as_str).collect::<Vec<_>>()),
    );
    spoke_reject(SpokeRejectCode::SourceRetired, message, Some(details))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A minimal wire-shaped entry decoded into the generated
    /// `KnowledgeEntry` type, optionally carrying `anchor_json` as its
    /// `adoption_anchor`.
    fn entry_with_anchor(entry_id: &str, anchor_json: Value) -> KnowledgeEntry {
        let mut wire = json!({
            "schema_version": 1,
            "entry_id": entry_id,
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": {},
            "extensions": {},
        });
        if !anchor_json.is_null() {
            wire["adoption_anchor"] = anchor_json;
        }
        serde_json::from_value(wire).expect("wire entry decodes")
    }

    fn anchor(source_revision: u64) -> Value {
        json!({
            "schema_version": 1,
            "source_entry_id": "ke_terminal",
            "source_scope_id": "scope_a",
            "source_revision": source_revision,
            "extensions": {},
        })
    }

    #[test]
    fn identical_canonical_anchor_passes_the_gate() {
        let stored = entry_with_anchor("ke_dst", anchor(3));
        let candidate = entry_with_anchor("ke_dst", anchor(3));
        assert!(matches!(
            assert_adoption_anchor_preserved(&candidate, Some(&stored)),
            SpokeResult::Ok(())
        ));
    }

    #[test]
    fn changed_anchor_refuses_immutable() {
        let stored = entry_with_anchor("ke_dst", anchor(3));
        let candidate = entry_with_anchor("ke_dst", anchor(4));
        match assert_adoption_anchor_preserved(&candidate, Some(&stored)) {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::AdoptionAnchorImmutable);
            }
            SpokeResult::Ok(()) => panic!("expected ADOPTION_ANCHOR_IMMUTABLE"),
        }
    }

    #[test]
    fn non_finite_number_cannot_canonicalize() {
        // serde_jcs refuses non-finite f64 — the `None` arm below is the
        // gate's reach of this fact.
        assert!(canonical_bytes(&f64::NAN).is_none());
        assert!(canonical_bytes(&f64::INFINITY).is_none());
    }

    #[test]
    fn two_failed_canonicalizations_do_not_prove_preservation() {
        // Regression pin: `None == None` used to pass as "unchanged".
        assert!(matches!(
            anchor_verdict(None, None),
            AnchorVerdict::InvalidInput
        ));
        assert!(matches!(
            anchor_verdict(None, Some(b"{}".to_vec())),
            AnchorVerdict::InvalidInput
        ));
        assert!(matches!(
            anchor_verdict(Some(b"{}".to_vec()), None),
            AnchorVerdict::InvalidInput
        ));
    }

    #[test]
    fn differing_canonical_bytes_are_immutable_not_invalid() {
        assert!(matches!(
            anchor_verdict(Some(b"1".to_vec()), Some(b"2".to_vec())),
            AnchorVerdict::Immutable
        ));
        assert!(matches!(
            anchor_verdict(Some(b"1".to_vec()), Some(b"1".to_vec())),
            AnchorVerdict::Preserved
        ));
    }
}
