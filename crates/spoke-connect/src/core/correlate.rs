//! Response correlation: echo checks for `session_id`, `sequence`, and
//! `request_id`.
//!
//! Normative rule (`.mstar/specs/spoke-connect.md` §Ordering semantics): a
//! response MUST echo `session_id`, `sequence`, and `request_id` from the
//! request; any mismatch is a correlation failure (local error).

use crate::core::error::CoreInvokeError;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::connect_invoke_response::ConnectInvokeResponse;

/// The minimal echo material needed to correlate a response with its
/// request: the request's wire echo fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correlation {
    pub session_id: String,
    pub sequence: i64,
    pub request_id: String,
}

impl From<&ConnectInvokeRequest> for Correlation {
    fn from(request: &ConnectInvokeRequest) -> Self {
        Self {
            session_id: request.session_id.to_string(),
            sequence: request.sequence,
            request_id: request.request_id.to_string(),
        }
    }
}

impl From<&ConnectInvokeResponse> for Correlation {
    fn from(response: &ConnectInvokeResponse) -> Self {
        match response {
            ConnectInvokeResponse::Variant0 {
                session_id,
                sequence,
                request_id,
                ..
            }
            | ConnectInvokeResponse::Variant1 {
                session_id,
                sequence,
                request_id,
                ..
            } => Self {
                session_id: session_id.to_string(),
                sequence: *sequence,
                request_id: request_id.to_string(),
            },
        }
    }
}

/// Check that `actual` (a response's echo fields) matches `expected` (the
/// request's echo fields) on all three fields.
pub fn check_response_correlation(
    expected: &Correlation,
    actual: &Correlation,
) -> Result<(), CoreInvokeError> {
    if expected.session_id != actual.session_id
        || expected.sequence != actual.sequence
        || expected.request_id != actual.request_id
    {
        return Err(CoreInvokeError::CorrelationMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{authenticate_invoke_request, authenticate_invoke_response};

    /// Deterministic 32-byte test identity seed (parameterized helper — no
    /// key material as a literal in a crypto call site, mirroring the
    /// envelope-auth unit-test convention).
    fn test_seed() -> [u8; 32] {
        [0x2b; 32]
    }

    fn request(sequence: i64, request_id: &str, session_id: &str) -> ConnectInvokeRequest {
        authenticate_invoke_request(
            &test_seed(),
            &crate::core::InvokeRequestSignInput {
                session_id: session_id.to_string(),
                sequence,
                request_id: request_id.to_string(),
                op: "check".to_string(),
                payload: serde_json::json!({ "findings": [] }),
                auth: None,
            },
            Default::default(),
        )
        .expect("test seed is 32 bytes — signing is infallible")
    }

    fn success_response(
        sequence: i64,
        request_id: &str,
        session_id: &str,
    ) -> ConnectInvokeResponse {
        authenticate_invoke_response(
            &test_seed(),
            &crate::core::InvokeResponseSignInput::Success {
                session_id: session_id.to_string(),
                sequence,
                request_id: request_id.to_string(),
                payload: serde_json::json!({ "findings": [] }),
            },
            Default::default(),
        )
        .expect("test seed is 32 bytes — signing is infallible")
    }

    fn error_response(sequence: i64, request_id: &str, session_id: &str) -> ConnectInvokeResponse {
        authenticate_invoke_response(
            &test_seed(),
            &crate::core::InvokeResponseSignInput::Error {
                session_id: session_id.to_string(),
                sequence,
                request_id: request_id.to_string(),
                error: spoke_schemas::connect::connect_invoke_response::ErrorEnvelope {
                    code: "check_failed".into(),
                    details: Default::default(),
                    extensions: Default::default(),
                    message: "spike check failed".into(),
                },
            },
            Default::default(),
        )
        .expect("test seed is 32 bytes — signing is infallible")
    }

    #[test]
    fn echo_mismatches_are_detected() {
        let expected = Correlation::from(&request(0, "req-1", "sess-1"));

        check_response_correlation(
            &expected,
            &Correlation::from(&success_response(0, "req-1", "sess-1")),
        )
        .expect("exact echo passes");

        // Wrong sequence.
        let err = check_response_correlation(
            &expected,
            &Correlation::from(&success_response(1, "req-1", "sess-1")),
        )
        .expect_err("sequence mismatch");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));

        // Wrong request_id.
        let err = check_response_correlation(
            &expected,
            &Correlation::from(&success_response(0, "other-req", "sess-1")),
        )
        .expect_err("request id mismatch");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));

        // Wrong session_id (error branch echoes too).
        let err = check_response_correlation(
            &expected,
            &Correlation::from(&error_response(0, "req-1", "other-session")),
        )
        .expect_err("session mismatch");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));
    }

    #[test]
    fn both_response_branches_expose_echo_fields() {
        let expected = Correlation::from(&request(3, "req-9", "sess-2"));
        check_response_correlation(
            &expected,
            &Correlation::from(&success_response(3, "req-9", "sess-2")),
        )
        .expect("success branch echo");
        check_response_correlation(
            &expected,
            &Correlation::from(&error_response(3, "req-9", "sess-2")),
        )
        .expect("error branch echo");
    }
}
