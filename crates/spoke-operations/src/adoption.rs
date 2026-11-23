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
/// branch — two uncanonicalizable anchors compare equal, byte parity).
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
    if let Some(candidate_anchor) = candidate.adoption_anchor.as_ref() {
        if canonical_bytes(candidate_anchor) == canonical_bytes(stored_anchor) {
            return spoke_ok_unit();
        }
    }
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
