//! Unified success/reject envelope for all SPOKE operations helpers.

use serde_json::{Map, Value};
use std::fmt;

/// Stable reject code strings — parity with `@42ch/spoke-operations` `SpokeRejectCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpokeRejectCode {
    InvalidInput,
    InvalidStatus,
    InvalidStatusTransition,
    CandidateNotProvisional,
    CandidateTerminalStatus,
    EmptyCanonicalName,
    MergeTargetSelf,
    MissingRequiredField,
    InvalidPacketInput,
    RevisionConflict,
    StoredRevisionStale,
    InvalidKnowledgeEntryStatus,
    InvalidKnowledgeEntryStatusTransition,
    DuplicateActiveKnowledgeEntry,
    KnowledgeEntryNotFound,
    KnowledgeEntryAlreadyExists,
    KnowledgeEntryTerminalStatus,
    RelationSelfEdge,
    RelationMissingEndpoint,
}

impl SpokeRejectCode {
    /// Wire string literal for this code (matches TypeScript `as const` values).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "INVALID_INPUT",
            Self::InvalidStatus => "INVALID_STATUS",
            Self::InvalidStatusTransition => "INVALID_STATUS_TRANSITION",
            Self::CandidateNotProvisional => "CANDIDATE_NOT_PROVISIONAL",
            Self::CandidateTerminalStatus => "CANDIDATE_TERMINAL_STATUS",
            Self::EmptyCanonicalName => "EMPTY_CANONICAL_NAME",
            Self::MergeTargetSelf => "MERGE_TARGET_SELF",
            Self::MissingRequiredField => "MISSING_REQUIRED_FIELD",
            Self::InvalidPacketInput => "INVALID_PACKET_INPUT",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::StoredRevisionStale => "STORED_REVISION_STALE",
            Self::InvalidKnowledgeEntryStatus => "INVALID_KNOWLEDGE_ENTRY_STATUS",
            Self::InvalidKnowledgeEntryStatusTransition => {
                "INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION"
            }
            Self::DuplicateActiveKnowledgeEntry => "DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY",
            Self::KnowledgeEntryNotFound => "KNOWLEDGE_ENTRY_NOT_FOUND",
            Self::KnowledgeEntryAlreadyExists => "KNOWLEDGE_ENTRY_ALREADY_EXISTS",
            Self::KnowledgeEntryTerminalStatus => "KNOWLEDGE_ENTRY_TERMINAL_STATUS",
            Self::RelationSelfEdge => "RELATION_SELF_EDGE",
            Self::RelationMissingEndpoint => "RELATION_MISSING_ENDPOINT",
        }
    }
}

impl fmt::Display for SpokeRejectCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured reject payload (`ok: false` in TypeScript).
#[derive(Debug, Clone, PartialEq)]
pub struct SpokeReject {
    pub code: SpokeRejectCode,
    pub message: String,
    pub details: Option<Map<String, Value>>,
}

/// Discriminated success/reject result (`SpokeResult` in TypeScript).
#[derive(Debug, Clone, PartialEq)]
pub enum SpokeResult<T> {
    Ok(T),
    Reject(SpokeReject),
}

impl<T> SpokeResult<T> {
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }

    #[must_use]
    pub const fn is_reject(&self) -> bool {
        matches!(self, Self::Reject(_))
    }
}

#[must_use]
pub fn spoke_ok<T>(value: T) -> SpokeResult<T> {
    SpokeResult::Ok(value)
}

#[must_use]
pub fn spoke_ok_unit() -> SpokeResult<()> {
    SpokeResult::Ok(())
}

#[must_use]
pub fn spoke_reject<T>(
    code: SpokeRejectCode,
    message: impl Into<String>,
    details: Option<Map<String, Value>>,
) -> SpokeResult<T> {
    SpokeResult::Reject(SpokeReject {
        code,
        message: message.into(),
        details,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_code_strings_match_typescript() {
        let codes = [
            SpokeRejectCode::InvalidInput,
            SpokeRejectCode::InvalidStatus,
            SpokeRejectCode::InvalidStatusTransition,
            SpokeRejectCode::CandidateNotProvisional,
            SpokeRejectCode::CandidateTerminalStatus,
            SpokeRejectCode::EmptyCanonicalName,
            SpokeRejectCode::MergeTargetSelf,
            SpokeRejectCode::MissingRequiredField,
            SpokeRejectCode::InvalidPacketInput,
            SpokeRejectCode::RevisionConflict,
            SpokeRejectCode::StoredRevisionStale,
            SpokeRejectCode::InvalidKnowledgeEntryStatus,
            SpokeRejectCode::InvalidKnowledgeEntryStatusTransition,
            SpokeRejectCode::DuplicateActiveKnowledgeEntry,
            SpokeRejectCode::KnowledgeEntryNotFound,
            SpokeRejectCode::KnowledgeEntryAlreadyExists,
            SpokeRejectCode::KnowledgeEntryTerminalStatus,
            SpokeRejectCode::RelationSelfEdge,
            SpokeRejectCode::RelationMissingEndpoint,
        ];
        assert_eq!(codes.len(), 19);
        for code in codes {
            assert!(!code.as_str().is_empty());
        }
    }
}
