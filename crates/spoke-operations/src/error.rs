//! Error envelope mapping helpers.

use crate::result::{SpokeReject, SpokeRejectCode};
use serde_json::{Map, Value};
use spoke_schemas::ErrorEnvelope;
use std::collections::HashMap;

/// Map SpokeReject to ops ErrorEnvelope wire shape.
#[must_use]
pub fn to_error_envelope(reject: &SpokeReject) -> ErrorEnvelope {
    ErrorEnvelope {
        code: reject.code.as_str().to_owned(),
        message: reject.message.clone(),
        details: reject.details.clone().unwrap_or_default(),
        extensions: HashMap::new(),
    }
}

/// Map ErrorEnvelope back to SpokeReject.
#[must_use]
pub fn from_error_envelope(error: &ErrorEnvelope) -> SpokeReject {
    let Some(code) = SpokeRejectCode::try_from_str(&error.code) else {
        return SpokeReject {
            code: SpokeRejectCode::InvalidInput,
            message: format!("Unknown error code: {}", error.code),
            details: Some(Map::from_iter([(
                "wire_code".into(),
                Value::String(error.code.clone()),
            )])),
        };
    };

    SpokeReject {
        code,
        message: error.message.clone(),
        details: if error.details.is_empty() {
            None
        } else {
            Some(error.details.clone())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::{spoke_reject, SpokeResult};
    use serde_json::json;

    const DEEPEN_REJECT_CODES: [SpokeRejectCode; 14] = [
        SpokeRejectCode::InvalidInput,
        SpokeRejectCode::MissingRequiredField,
        SpokeRejectCode::RevisionConflict,
        SpokeRejectCode::StoredRevisionStale,
        SpokeRejectCode::KnowledgeEntryNotFound,
        SpokeRejectCode::KnowledgeEntryAlreadyExists,
        SpokeRejectCode::KnowledgeEntryTerminalStatus,
        SpokeRejectCode::RelationSelfEdge,
        SpokeRejectCode::RelationMissingEndpoint,
        SpokeRejectCode::RelationNotFound,
        SpokeRejectCode::RelationAlreadyExists,
        SpokeRejectCode::DuplicateActiveKnowledgeEntry,
        SpokeRejectCode::CapabilityPortMissing,
        SpokeRejectCode::InternalError,
    ];

    const FIRST_SLICE_REJECT_CODES: [SpokeRejectCode; 9] = [
        SpokeRejectCode::InvalidStatus,
        SpokeRejectCode::InvalidStatusTransition,
        SpokeRejectCode::CandidateNotProvisional,
        SpokeRejectCode::CandidateTerminalStatus,
        SpokeRejectCode::EmptyCanonicalName,
        SpokeRejectCode::MergeTargetSelf,
        SpokeRejectCode::InvalidPacketInput,
        SpokeRejectCode::InvalidKnowledgeEntryStatus,
        SpokeRejectCode::InvalidKnowledgeEntryStatusTransition,
    ];

    fn reject_from_code(code: SpokeRejectCode) -> SpokeReject {
        match spoke_reject::<()>(
            code,
            format!("message for {}", code.as_str()),
            Some(Map::from_iter([("sample".into(), json!(true))])),
        ) {
            SpokeResult::Reject(reject) => reject,
            SpokeResult::Ok(_) => panic!("expected reject"),
        }
    }

    #[test]
    fn round_trips_all_documented_codes() {
        for code in FIRST_SLICE_REJECT_CODES
            .into_iter()
            .chain(DEEPEN_REJECT_CODES.into_iter())
        {
            let reject = reject_from_code(code);
            let envelope = to_error_envelope(&reject);

            assert!(envelope.extensions.is_empty());
            assert_eq!(envelope.code, code.as_str());
            assert_eq!(envelope.message, format!("message for {}", code.as_str()));
            assert_eq!(
                envelope.details,
                Map::from_iter([("sample".into(), json!(true))])
            );

            assert_eq!(from_error_envelope(&envelope), reject);
        }
    }

    #[test]
    fn omits_details_when_absent_on_outbound_map() {
        let reject = match spoke_reject::<()>(
            SpokeRejectCode::InvalidInput,
            "no details",
            None,
        ) {
            SpokeResult::Reject(reject) => reject,
            SpokeResult::Ok(_) => panic!("expected reject"),
        };

        let envelope = to_error_envelope(&reject);
        assert!(envelope.details.is_empty());
        assert_eq!(from_error_envelope(&envelope), reject);
    }

    #[test]
    fn rejects_unknown_wire_error_codes_with_invalid_input() {
        let envelope = ErrorEnvelope {
            code: "NOT_A_SPOKE_CODE".into(),
            message: "upstream error".into(),
            details: Map::new(),
            extensions: HashMap::new(),
        };

        let result = from_error_envelope(&envelope);
        assert_eq!(result.code, SpokeRejectCode::InvalidInput);
        assert_eq!(result.message, "Unknown error code: NOT_A_SPOKE_CODE");
        assert_eq!(
            result.details,
            Some(Map::from_iter([(
                "wire_code".into(),
                json!("NOT_A_SPOKE_CODE")
            )]))
        );
    }
}
