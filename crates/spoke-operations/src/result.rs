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
    RelationNotFound,
    RelationAlreadyExists,
    CapabilityPortMissing,
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
            Self::RelationNotFound => "RELATION_NOT_FOUND",
            Self::RelationAlreadyExists => "RELATION_ALREADY_EXISTS",
            Self::CapabilityPortMissing => "CAPABILITY_PORT_MISSING",
        }
    }

    /// Parse a wire code string into a known reject code.
    #[must_use]
    pub fn try_from_str(code: &str) -> Option<Self> {
        match code {
            "INVALID_INPUT" => Some(Self::InvalidInput),
            "INVALID_STATUS" => Some(Self::InvalidStatus),
            "INVALID_STATUS_TRANSITION" => Some(Self::InvalidStatusTransition),
            "CANDIDATE_NOT_PROVISIONAL" => Some(Self::CandidateNotProvisional),
            "CANDIDATE_TERMINAL_STATUS" => Some(Self::CandidateTerminalStatus),
            "EMPTY_CANONICAL_NAME" => Some(Self::EmptyCanonicalName),
            "MERGE_TARGET_SELF" => Some(Self::MergeTargetSelf),
            "MISSING_REQUIRED_FIELD" => Some(Self::MissingRequiredField),
            "INVALID_PACKET_INPUT" => Some(Self::InvalidPacketInput),
            "REVISION_CONFLICT" => Some(Self::RevisionConflict),
            "STORED_REVISION_STALE" => Some(Self::StoredRevisionStale),
            "INVALID_KNOWLEDGE_ENTRY_STATUS" => Some(Self::InvalidKnowledgeEntryStatus),
            "INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION" => {
                Some(Self::InvalidKnowledgeEntryStatusTransition)
            }
            "DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY" => Some(Self::DuplicateActiveKnowledgeEntry),
            "KNOWLEDGE_ENTRY_NOT_FOUND" => Some(Self::KnowledgeEntryNotFound),
            "KNOWLEDGE_ENTRY_ALREADY_EXISTS" => Some(Self::KnowledgeEntryAlreadyExists),
            "KNOWLEDGE_ENTRY_TERMINAL_STATUS" => Some(Self::KnowledgeEntryTerminalStatus),
            "RELATION_SELF_EDGE" => Some(Self::RelationSelfEdge),
            "RELATION_MISSING_ENDPOINT" => Some(Self::RelationMissingEndpoint),
            "RELATION_NOT_FOUND" => Some(Self::RelationNotFound),
            "RELATION_ALREADY_EXISTS" => Some(Self::RelationAlreadyExists),
            "CAPABILITY_PORT_MISSING" => Some(Self::CapabilityPortMissing),
            _ => None,
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
        const TS_SPOKE_REJECT_CODES: &[&str] = &[
            "INVALID_INPUT",
            "INVALID_STATUS",
            "INVALID_STATUS_TRANSITION",
            "CANDIDATE_NOT_PROVISIONAL",
            "CANDIDATE_TERMINAL_STATUS",
            "EMPTY_CANONICAL_NAME",
            "MERGE_TARGET_SELF",
            "MISSING_REQUIRED_FIELD",
            "INVALID_PACKET_INPUT",
            "REVISION_CONFLICT",
            "STORED_REVISION_STALE",
            "INVALID_KNOWLEDGE_ENTRY_STATUS",
            "INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION",
            "DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY",
            "KNOWLEDGE_ENTRY_NOT_FOUND",
            "KNOWLEDGE_ENTRY_ALREADY_EXISTS",
            "KNOWLEDGE_ENTRY_TERMINAL_STATUS",
            "RELATION_SELF_EDGE",
            "RELATION_MISSING_ENDPOINT",
            "RELATION_NOT_FOUND",
            "RELATION_ALREADY_EXISTS",
            "CAPABILITY_PORT_MISSING",
        ];

        let rust_codes = [
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
            SpokeRejectCode::RelationNotFound,
            SpokeRejectCode::RelationAlreadyExists,
            SpokeRejectCode::CapabilityPortMissing,
        ];

        assert_eq!(TS_SPOKE_REJECT_CODES.len(), 22);
        assert_eq!(rust_codes.len(), TS_SPOKE_REJECT_CODES.len());

        for (code, expected) in rust_codes.into_iter().zip(TS_SPOKE_REJECT_CODES.iter()) {
            assert_eq!(code.as_str(), *expected);
        }
    }
}
