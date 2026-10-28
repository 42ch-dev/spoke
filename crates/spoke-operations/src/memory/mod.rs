//! Memory-system pure helpers — per-op request validators, selection
//! invariants, grant-coverage evaluation, freeze-allowlist predicate, and
//! JCS/SHA-256 digest helpers (spoke-memory protocol §§6.5–6.13, 7, 8.2).
//!
//! The generated wire types already enforce everything JSON Schema can
//! express (field presence, `minItems`, string patterns, `oneOf` shapes,
//! `NonZeroU64` bounds). These helpers cover the residual conditional rules
//! the codegen toolchain cannot derive (draft-07 `if`/`then`/`else`) plus the
//! pure policy predicates providers evaluate at admission.

pub mod digests;
pub mod grants;
pub mod validate;

pub use digests::{jcs_sha256_hex, jcs_sha256_hex_excluding};
pub use grants::{grant_covers, validate_grant_issuance, GrantRequirement, IssuedGrant};
pub use validate::{
    content_wire_bytes, is_freeze_allowlisted, validate_memory_candidates_list,
    validate_memory_candidates_review, validate_memory_capture, validate_memory_grants_grant,
    validate_memory_grants_list, validate_memory_imports_stage, validate_memory_recall,
    validate_memory_records_create,
    validate_memory_records_list, validate_memory_records_update,
    validate_memory_reflections_generate, validate_memory_reflections_list,
    validate_memory_reflections_state, validate_memory_spaces_list,
    validate_memory_subjects_fence,
};

#[cfg(test)]
mod tests {
    use crate::{MemoryPorts, SpokeRejectCode};

    /// Wire-string round-trip for every §14.1 memory reject code.
    #[test]
    fn memory_reject_codes_wire_round_trip() {
        const MEMORY_CODES: &[(SpokeRejectCode, &str)] = &[
            (SpokeRejectCode::NotFound, "NOT_FOUND"),
            (SpokeRejectCode::NotAuthorized, "NOT_AUTHORIZED"),
            (SpokeRejectCode::GrantRevoked, "GRANT_REVOKED"),
            (SpokeRejectCode::SpaceFrozen, "SPACE_FROZEN"),
            (SpokeRejectCode::SubjectFenced, "SUBJECT_FENCED"),
            (SpokeRejectCode::CandidateConsumed, "CANDIDATE_CONSUMED"),
            (SpokeRejectCode::ProfileIncomplete, "PROFILE_INCOMPLETE"),
            (SpokeRejectCode::InsufficientData, "INSUFFICIENT_DATA"),
            (SpokeRejectCode::IdempotencyConflict, "IDEMPOTENCY_CONFLICT"),
            (SpokeRejectCode::OutcomeUncertain, "OUTCOME_UNCERTAIN"),
            (SpokeRejectCode::ExportExpired, "EXPORT_EXPIRED"),
            (SpokeRejectCode::ImportConflict, "IMPORT_CONFLICT"),
            (SpokeRejectCode::ReceiptExpired, "RECEIPT_EXPIRED"),
            (SpokeRejectCode::CursorInvalid, "CURSOR_INVALID"),
        ];

        assert_eq!(MEMORY_CODES.len(), 14);
        for (code, wire) in MEMORY_CODES {
            assert_eq!(code.as_str(), *wire);
            assert_eq!(SpokeRejectCode::try_from_str(wire), Some(*code));
        }
    }

    /// `Box<dyn MemoryPort>` must compile — the trait stays object-safe
    /// (`#[async_trait]` boxes the futures, as with the baseline ports).
    #[test]
    fn memory_port_trait_object_is_object_safe() {
        let _port: Option<Box<dyn crate::MemoryPort>> = None;
    }

    struct MemorylessAdapter;

    impl MemoryPorts for MemorylessAdapter {}

    /// The availability probe defaults to `None` for adapters that do not
    /// serve the memory family (mirrors TS `requirePortMethod` miss).
    #[test]
    fn memory_ports_probe_defaults_to_none() {
        let adapter = MemorylessAdapter;
        assert!(adapter.as_memory().is_none());
    }
}
