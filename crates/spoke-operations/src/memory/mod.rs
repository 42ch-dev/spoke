//! Memory-system pure helpers (validators, grant coverage, digests) — P2 Task 2.
//! Task 1 seeds this module with the memory port / reject-code contract tests.

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
