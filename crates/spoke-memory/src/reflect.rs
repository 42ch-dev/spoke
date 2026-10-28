//! Deterministic reflection basis digests, staleness, and assembly (§7.25–7.27).

use serde_json::{json, Map, Value};
use spoke_schemas::data::provenance_filter::ProvenanceFilter;
use spoke_schemas::data::read_selection::ReadSelection;

use crate::transfer::{canonical_json_bytes, sha256_hex};
use crate::ENGINE_POLICY_VERSION;

/// One source record pinned by a reflection basis, by identity and revision.
pub struct SourceRef<'a> {
    pub record_id: &'a str,
    pub revision: u64,
}

/// The pinned inputs a reflection is generated from (§6.9 `basis`, §7.25).
pub struct ReflectionBasis<'a> {
    pub selection: &'a ReadSelection,
    pub filter: Option<&'a ProvenanceFilter>,
    /// Authorized source records; order-independent.
    pub sources: &'a [SourceRef<'a>],
    /// Profile kind → revision.
    pub profile_revisions: &'a [(&'a str, u64)],
}

/// The two digests a basis pins: `source_set_digest` (§6.9) and
/// `state_digest` (§7.25, the CAS token for generation).
pub struct BasisDigests {
    pub source_set_digest: String,
    pub state_digest: String,
}

/// Deterministic placeholder reflection produced without any model call.
pub struct ReflectionDraft {
    pub state_digest: String,
    pub source_set_digest: String,
    pub record_count: usize,
    pub format: &'static str,
    pub body: String,
}

/// Both basis digests; identical inputs always yield identical digests.
pub fn basis_digests(basis: &ReflectionBasis<'_>) -> BasisDigests {
    let source_set_digest = sha256_hex(
        &canonical_json_bytes(&Value::Array(source_entries(basis)))
            .expect("a source set is JSON-serializable by construction"),
    );
    BasisDigests {
        source_set_digest,
        state_digest: state_digest_with_policy_version(basis, ENGINE_POLICY_VERSION),
    }
}

/// `true` iff the current basis differs from the pinned basis (§6.9, MEM-R013).
pub fn is_stale(pinned: &ReflectionBasis<'_>, current: &ReflectionBasis<'_>) -> bool {
    state_digest_with_policy_version(pinned, ENGINE_POLICY_VERSION)
        != state_digest_with_policy_version(current, ENGINE_POLICY_VERSION)
}

/// Pure assembly hook: a basis-derived body skeleton, no model, no I/O.
///
/// Policy: the placeholder body is markdown that names the pinned digests, the
/// record count and the engine policy version; it is deterministic for a given
/// basis and carries no record content.
pub fn assemble_reflection(basis: &ReflectionBasis<'_>) -> ReflectionDraft {
    let digests = basis_digests(basis);
    let mut record_ids: Vec<&str> = basis
        .sources
        .iter()
        .map(|source| source.record_id)
        .collect();
    record_ids.sort_unstable();
    record_ids.dedup();
    let body = format!(
        "# Reflection\n\n\
         - engine_policy_version: {ENGINE_POLICY_VERSION}\n\
         - state_digest: {}\n\
         - source_set_digest: {}\n\
         - records: {}\n\
         - sources: {}\n",
        digests.state_digest,
        digests.source_set_digest,
        basis.sources.len(),
        record_ids.join(", "),
    );
    ReflectionDraft {
        state_digest: digests.state_digest,
        source_set_digest: digests.source_set_digest,
        record_count: basis.sources.len(),
        format: "markdown",
        body,
    }
}

/// The exact JSON object the `state_digest` commits to (§7.25).
fn digest_object(basis: &ReflectionBasis<'_>, policy_version: &str) -> Value {
    let profile_revisions: Map<String, Value> = basis
        .profile_revisions
        .iter()
        .map(|(kind, revision)| ((*kind).to_string(), json!(revision)))
        .collect();
    json!({
        "engine_policy_version": policy_version,
        "filter": basis.filter.map_or(Value::Null, |filter| json!(filter)),
        "profile_revisions": profile_revisions,
        "selection": json!(basis.selection),
        "sources": source_entries(basis),
    })
}

fn state_digest_with_policy_version(basis: &ReflectionBasis<'_>, policy_version: &str) -> String {
    sha256_hex(
        &canonical_json_bytes(&digest_object(basis, policy_version))
            .expect("a basis is JSON-serializable by construction"),
    )
}

/// Source records sorted by id then revision — the digest is order-independent.
fn source_entries(basis: &ReflectionBasis<'_>) -> Vec<Value> {
    let mut sources: Vec<&SourceRef<'_>> = basis.sources.iter().collect();
    sources.sort_by(|left, right| {
        left.record_id
            .cmp(right.record_id)
            .then(left.revision.cmp(&right.revision))
    });
    sources
        .iter()
        .map(|source| json!({"record_id": source.record_id, "revision": source.revision}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn sources<'a>(items: &'a [(&'a str, u64)]) -> Vec<SourceRef<'a>> {
        items
            .iter()
            .map(|(record_id, revision)| SourceRef {
                record_id,
                revision: *revision,
            })
            .collect()
    }

    fn shared_only() -> ReadSelection {
        ReadSelection::SharedOnly
    }

    fn selected_contexts() -> ReadSelection {
        ReadSelection::SelectedContexts {
            context_ids: vec!["bnd_mira_harbor_run_7".to_string()],
            include_shared: Some(true),
        }
    }

    fn filter() -> ProvenanceFilter {
        ProvenanceFilter {
            source_context_ids: vec!["ctx_harbor".to_string()],
        }
    }

    #[test]
    fn digests_are_stable_and_hex() {
        let selection = shared_only();
        let pinned = vec![("rec_a", 1u64), ("rec_b", 2)];
        let source_refs = sources(&pinned);
        let profiles = vec![("personality", 3u64), ("preferences", 1)];
        let basis = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &source_refs,
            profile_revisions: &profiles,
        };

        let first = basis_digests(&basis);
        let second = basis_digests(&basis);
        assert_eq!(first.state_digest, second.state_digest);
        assert_eq!(first.source_set_digest, second.source_set_digest);
        assert_eq!(first.state_digest.len(), 64);
        assert!(first.state_digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(first.source_set_digest.len(), 64);
    }

    #[test]
    fn state_digest_pins_known_vector() {
        let selection = selected_contexts();
        let pinned = vec![("rec_a", 1u64), ("rec_b", 2)];
        let source_refs = sources(&pinned);
        let profiles = vec![("personality", 3u64)];
        let provenance = filter();
        let basis = ReflectionBasis {
            selection: &selection,
            filter: Some(&provenance),
            sources: &source_refs,
            profile_revisions: &profiles,
        };
        assert_eq!(
            basis_digests(&basis).state_digest,
            "257cfe0189524b049508a95d76484e797e2e1d75fd24eecb3610d72bd5ef47cc"
        );
    }

    #[test]
    fn source_set_digest_ignores_input_order() {
        let selection = shared_only();
        let forward = vec![("rec_a", 1u64), ("rec_b", 2)];
        let reversed = vec![("rec_b", 2u64), ("rec_a", 1)];
        let forward_refs = sources(&forward);
        let reversed_refs = sources(&reversed);
        let profiles: Vec<(&str, u64)> = Vec::new();
        let left = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &forward_refs,
            profile_revisions: &profiles,
        };
        let right = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &reversed_refs,
            profile_revisions: &profiles,
        };
        assert_eq!(
            basis_digests(&left).source_set_digest,
            basis_digests(&right).source_set_digest
        );
        assert_eq!(
            basis_digests(&left).state_digest,
            basis_digests(&right).state_digest
        );
    }

    #[test]
    fn staleness_flips_on_every_basis_movement() {
        let selection = shared_only();
        let other_selection = selected_contexts();
        let provenance = filter();
        let base_records = vec![("rec_a", 1u64)];
        let base_refs = sources(&base_records);
        let base_profiles = vec![("personality", 1u64)];
        let pinned = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &base_refs,
            profile_revisions: &base_profiles,
        };

        // Unchanged basis is not stale.
        assert!(!is_stale(&pinned, &pinned));

        // Record added.
        let added_records = vec![("rec_a", 1u64), ("rec_b", 1)];
        let added_refs = sources(&added_records);
        let added = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &added_refs,
            profile_revisions: &base_profiles,
        };
        assert!(is_stale(&pinned, &added));

        // Record revision bumped.
        let bumped_records = vec![("rec_a", 2u64)];
        let bumped_refs = sources(&bumped_records);
        let bumped = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &bumped_refs,
            profile_revisions: &base_profiles,
        };
        assert!(is_stale(&pinned, &bumped));

        // Record removed.
        let empty_records: Vec<(&str, u64)> = Vec::new();
        let empty_refs = sources(&empty_records);
        let removed = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &empty_refs,
            profile_revisions: &base_profiles,
        };
        assert!(is_stale(&pinned, &removed));

        // Profile revision bumped.
        let bumped_profiles = vec![("personality", 2u64)];
        let profile_moved = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &base_refs,
            profile_revisions: &bumped_profiles,
        };
        assert!(is_stale(&pinned, &profile_moved));

        // Selection changed.
        let selection_moved = ReflectionBasis {
            selection: &other_selection,
            filter: None,
            sources: &base_refs,
            profile_revisions: &base_profiles,
        };
        assert!(is_stale(&pinned, &selection_moved));

        // Filter added.
        let filter_moved = ReflectionBasis {
            selection: &selection,
            filter: Some(&provenance),
            sources: &base_refs,
            profile_revisions: &base_profiles,
        };
        assert!(is_stale(&pinned, &filter_moved));
    }

    #[test]
    fn policy_version_change_moves_the_state_digest() {
        let selection = shared_only();
        let records = vec![("rec_a", 1u64)];
        let source_refs = sources(&records);
        let profiles: Vec<(&str, u64)> = Vec::new();
        let basis = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &source_refs,
            profile_revisions: &profiles,
        };
        let current = state_digest_with_policy_version(&basis, ENGINE_POLICY_VERSION);
        let older = state_digest_with_policy_version(&basis, "spoke-memory/0.9.0-deterministic");
        assert_ne!(current, older);
        assert_eq!(current, basis_digests(&basis).state_digest);
    }

    #[test]
    fn assembly_reports_basis_and_policy_without_model() {
        let selection = shared_only();
        let records = vec![("rec_b", 2u64), ("rec_a", 1)];
        let source_refs = sources(&records);
        let profiles = vec![("personality", 3u64)];
        let basis = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &source_refs,
            profile_revisions: &profiles,
        };
        let draft = assemble_reflection(&basis);
        let digests = basis_digests(&basis);
        assert_eq!(draft.format, "markdown");
        assert_eq!(draft.record_count, 2);
        assert_eq!(draft.state_digest, digests.state_digest);
        assert_eq!(draft.source_set_digest, digests.source_set_digest);
        assert!(draft.body.contains(ENGINE_POLICY_VERSION));
        assert!(draft.body.contains(&digests.state_digest));
        // Deterministic: assembling twice yields the same body.
        assert_eq!(draft.body, assemble_reflection(&basis).body);
    }

    #[test]
    fn digest_object_shape_is_canonical() {
        let selection = shared_only();
        let records = vec![("rec_a", 1u64)];
        let source_refs = sources(&records);
        let profiles = vec![("personality", 3u64)];
        let basis = ReflectionBasis {
            selection: &selection,
            filter: None,
            sources: &source_refs,
            profile_revisions: &profiles,
        };
        let expected: Value = json!({
            "engine_policy_version": ENGINE_POLICY_VERSION,
            "filter": Value::Null,
            "profile_revisions": {"personality": 3},
            "selection": {"mode": "shared_only"},
            "sources": [{"record_id": "rec_a", "revision": 1}],
        });
        assert_eq!(digest_object(&basis, ENGINE_POLICY_VERSION), expected);
    }
}
