//! Integration tests for `orchestrate_extract`.
//!
//! Parity with `packages/spoke-operations/src/adapter/extract.test.ts`. The
//! frozen port parameter is `&dyn ExtractionPort`, so port absence is
//! unrepresentable at the type level; the `MissingExtractionPort` negative
//! double pins the same `CAPABILITY_PORT_MISSING` reject (with
//! `details.capability = "ke-extraction"`) the TS runtime guard produces.
//!
//! Fixtures are built through wire JSON, matching the crate's existing
//! serde-bridging convention for generated nominal types.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use spoke_operations::{
    orchestrate_extract, spoke_ok, spoke_reject, ExtractRunInput, ExtractionPort, ExtractionResult,
    SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::{ExtractRequest, ExtractResponse, KnowledgeEntry};
use std::future::{ready, Future};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

fn wire<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("extract fixture")
}

fn anchor() -> Value {
    json!({ "schema_version": 1, "source_id": "chapter-01", "extensions": {} })
}

fn request_json(run_id: &str, sources: Value) -> Value {
    json!({ "run_id": run_id, "sources": sources })
}

fn request_with(run_id: &str) -> ExtractRequest {
    wire(request_json(run_id, json!([anchor()])))
}

fn make_request() -> ExtractRequest {
    request_with("run_001")
}

fn candidate_json(entry_id: &str, status: &str) -> Value {
    json!({
        "schema_version": 1,
        "entry_id": entry_id,
        "entry_type": "character",
        "canonical_name": "Mira Vale",
        "status": status,
        "body": { "summary": "Protagonist" },
        "extensions": {},
    })
}

fn candidate(entry_id: &str, status: &str) -> KnowledgeEntry {
    wire(candidate_json(entry_id, status))
}

fn result_of(candidates: Vec<KnowledgeEntry>) -> ExtractionResult {
    ExtractionResult {
        candidates,
        method: None,
        coverage_hint: None,
    }
}

fn reject_of(result: SpokeResult<Value>) -> SpokeReject {
    match result {
        SpokeResult::Reject(reject) => reject,
        SpokeResult::Ok(_) => panic!("expected a reject fixture"),
    }
}

/// Port double that records the requests handed to the loader.
struct RecordingPort {
    outcome: SpokeResult<Value>,
    calls: Mutex<Vec<Value>>,
}

impl RecordingPort {
    fn new(outcome: SpokeResult<Value>) -> Self {
        Self {
            outcome,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Value> {
        self.calls.lock().expect("port call log").clone()
    }
}

#[async_trait]
impl ExtractionPort for RecordingPort {
    async fn load_extraction_input(&self, request: &ExtractRequest) -> SpokeResult<Value> {
        self.calls
            .lock()
            .expect("port call log")
            .push(serde_json::to_value(request).expect("wire request"));
        self.outcome.clone()
    }
}

/// Future that reports `Pending` once (waking itself) before resolving, so a
/// caller that drops or short-circuits the await observes one poll instead of
/// the completed value.
struct PendingOnce<F> {
    resolve: Option<F>,
    polls: Arc<AtomicUsize>,
}

impl<F, T> Future for PendingOnce<F>
where
    F: FnOnce() -> T + Unpin,
{
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let this = self.get_mut();
        if this.polls.fetch_add(1, Ordering::SeqCst) == 0 {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let resolve = this.resolve.take().expect("polled after completion");
        Poll::Ready(resolve())
    }
}

#[test]
fn rejects_an_empty_source_list_before_any_port_or_extractor_call() {
    struct UnreachablePort;

    #[async_trait]
    impl ExtractionPort for UnreachablePort {
        async fn load_extraction_input(&self, _request: &ExtractRequest) -> SpokeResult<Value> {
            panic!("the loader must not be reached");
        }
    }

    let extractor_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&extractor_calls);

    let result = pollster::block_on(orchestrate_extract(
        &UnreachablePort,
        wire(request_json("run_001", json!([]))),
        move |_input: ExtractRunInput| {
            calls.fetch_add(1, Ordering::SeqCst);
            ready(spoke_ok(result_of(Vec::new())))
        },
    ));

    assert_eq!(extractor_calls.load(Ordering::SeqCst), 0);
    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            let details = reject.details.expect("details present");
            assert_eq!(details.get("field").and_then(Value::as_str), Some("sources"));
        }
        other => panic!("expected an INVALID_INPUT reject, got: {other:?}"),
    }
}

#[test]
fn rejects_an_empty_run_id_at_the_wire_boundary() {
    // The generated `ExtractRequestRunId` is the Rust half of the TS run_id
    // gate: an empty correlation id cannot even be constructed.
    assert!(serde_json::from_value::<ExtractRequest>(request_json("", json!([anchor()]))).is_err());
}

#[test]
fn returns_the_load_rejection_unchanged_and_never_invokes_the_extractor() {
    let failure = reject_of(spoke_reject::<Value>(
        SpokeRejectCode::InternalError,
        "source store unavailable",
        None,
    ));
    let port = RecordingPort::new(SpokeResult::Reject(failure.clone()));
    let extractor_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&extractor_calls);

    let result = pollster::block_on(orchestrate_extract(
        &port,
        make_request(),
        move |_input: ExtractRunInput| {
            calls.fetch_add(1, Ordering::SeqCst);
            ready(spoke_ok(result_of(Vec::new())))
        },
    ));

    assert_eq!(extractor_calls.load(Ordering::SeqCst), 0);
    match result {
        SpokeResult::Reject(reject) => assert_eq!(reject, failure),
        other => panic!("expected the load reject, got: {other:?}"),
    }
}

#[test]
fn returns_the_extractor_rejection_unchanged() {
    let failure = reject_of(spoke_reject::<Value>(
        SpokeRejectCode::InternalError,
        "extraction backend down",
        None,
    ));
    let port = RecordingPort::new(spoke_ok(json!({ "chapter": "text" })));
    let produce = failure.clone();

    let result = pollster::block_on(orchestrate_extract(
        &port,
        make_request(),
        move |_input: ExtractRunInput| ready(SpokeResult::<ExtractionResult>::Reject(produce)),
    ));

    assert_eq!(port.calls().len(), 1);
    match result {
        SpokeResult::Reject(reject) => assert_eq!(reject, failure),
        other => panic!("expected the extractor reject, got: {other:?}"),
    }
}

#[test]
fn awaits_pending_load_and_extractor_futures_and_echoes_the_run_id() {
    struct PendingLoadPort {
        loaded: Value,
        polls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ExtractionPort for PendingLoadPort {
        async fn load_extraction_input(&self, _request: &ExtractRequest) -> SpokeResult<Value> {
            let loaded = self.loaded.clone();
            PendingOnce {
                resolve: Some(move || spoke_ok(loaded)),
                polls: Arc::clone(&self.polls),
            }
            .await
        }
    }

    let loaded = json!({ "chapter": "raw manuscript text" });
    let request = make_request();
    let request_wire = serde_json::to_value(&request).expect("wire request");
    let load_polls = Arc::new(AtomicUsize::new(0));
    let extract_polls = Arc::new(AtomicUsize::new(0));
    let extract_polls_handle = Arc::clone(&extract_polls);
    let extractor_calls = Arc::new(AtomicUsize::new(0));

    let port = PendingLoadPort {
        loaded: loaded.clone(),
        polls: Arc::clone(&load_polls),
    };
    let calls = Arc::clone(&extractor_calls);
    let expected_input = loaded.clone();
    let expected_candidate = candidate("kb_extract_1", "provisional");

    let result = pollster::block_on(orchestrate_extract(
        &port,
        request,
        move |input: ExtractRunInput| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                serde_json::to_value(&input.request).expect("wire request"),
                request_wire
            );
            assert_eq!(input.input, expected_input);
            let candidate = expected_candidate.clone();
            PendingOnce {
                resolve: Some(move || spoke_ok(result_of(vec![candidate]))),
                polls: Arc::clone(&extract_polls_handle),
            }
        },
    ));

    // Two polls on each future: the orchestrator waited for the pending stage
    // instead of resolving early or dropping the callbacks.
    assert_eq!(load_polls.load(Ordering::SeqCst), 2);
    assert_eq!(extract_polls.load(Ordering::SeqCst), 2);
    assert_eq!(extractor_calls.load(Ordering::SeqCst), 1);

    match result {
        SpokeResult::Ok(ExtractResponse::Variant0 { candidates, run, .. }) => {
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].entry_id, "kb_extract_1");
            assert_eq!(candidates[0].status, "provisional");
            assert_eq!(run.run_id.as_str(), "run_001");
        }
        other => panic!("expected an extraction success response, got: {other:?}"),
    }
}

#[test]
fn rejects_the_whole_set_when_a_candidate_has_a_terminal_status() {
    let port = RecordingPort::new(spoke_ok(json!({})));

    let result = pollster::block_on(orchestrate_extract(
        &port,
        make_request(),
        move |_input: ExtractRunInput| {
            ready(spoke_ok(result_of(vec![
                candidate("kb_ok", "provisional"),
                candidate("kb_deleted", "deleted"),
            ])))
        },
    ));

    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CandidateTerminalStatus);
            let details = reject.details.expect("details present");
            assert_eq!(
                details.get("status").and_then(Value::as_str),
                Some("deleted")
            );
            assert_eq!(
                details.get("entry_id").and_then(Value::as_str),
                Some("kb_deleted")
            );
        }
        other => panic!("expected a terminal-status reject, got: {other:?}"),
    }
}

#[test]
fn rejects_the_whole_set_when_a_candidate_is_not_provisional() {
    let port = RecordingPort::new(spoke_ok(json!({})));

    let result = pollster::block_on(orchestrate_extract(
        &port,
        make_request(),
        move |_input: ExtractRunInput| {
            ready(spoke_ok(result_of(vec![
                candidate("kb_ok", "provisional"),
                candidate("kb_confirmed", "confirmed"),
            ])))
        },
    ));

    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CandidateNotProvisional);
            let details = reject.details.expect("details present");
            assert_eq!(
                details.get("status").and_then(Value::as_str),
                Some("confirmed")
            );
            assert_eq!(
                details.get("entry_id").and_then(Value::as_str),
                Some("kb_confirmed")
            );
        }
        other => panic!("expected a not-provisional reject, got: {other:?}"),
    }
}

#[test]
fn returns_a_successful_empty_candidate_set_when_the_extractor_finds_nothing() {
    let port = RecordingPort::new(spoke_ok(json!({ "chapter": "text" })));

    let result = pollster::block_on(orchestrate_extract(
        &port,
        make_request(),
        move |_input: ExtractRunInput| ready(spoke_ok(result_of(Vec::new()))),
    ));

    // The loader saw the caller's request exactly once.
    assert_eq!(
        port.calls(),
        vec![serde_json::to_value(make_request()).expect("wire request")]
    );
    match result {
        SpokeResult::Ok(response) => assert_eq!(
            serde_json::to_value(&response).expect("wire response"),
            json!({ "candidates": [], "run": { "run_id": "run_001" } })
        ),
        other => panic!("expected an extraction success response, got: {other:?}"),
    }
}

#[test]
fn echoes_the_caller_run_id_and_retains_opaque_advisory_metadata_verbatim() {
    let coverage_hint = json!(["chapter-01", { "ratio": 0.5 }]);
    let port = RecordingPort::new(spoke_ok(json!({ "chapter": "text" })));
    let hint = coverage_hint.clone();
    let extracted = candidate("kb_opaque", "provisional");
    let expected = candidate_json("kb_opaque", "provisional");

    let result = pollster::block_on(orchestrate_extract(
        &port,
        request_with("run_opaque"),
        move |_input: ExtractRunInput| {
            ready(spoke_ok(ExtractionResult {
                candidates: vec![extracted],
                method: Some("rule-based-v1".to_owned()),
                coverage_hint: Some(hint),
            }))
        },
    ));

    match result {
        SpokeResult::Ok(response) => assert_eq!(
            serde_json::to_value(&response).expect("wire response"),
            json!({
                "candidates": [expected],
                "run": {
                    "run_id": "run_opaque",
                    "method": "rule-based-v1",
                    "coverage_hint": coverage_hint,
                },
            })
        ),
        other => panic!("expected an extraction success response, got: {other:?}"),
    }
}

#[test]
fn emits_no_method_key_when_the_extractor_returns_an_empty_method() {
    let port = RecordingPort::new(spoke_ok(json!({ "chapter": "text" })));

    let result = pollster::block_on(orchestrate_extract(
        &port,
        request_with("run_method_empty"),
        move |_input: ExtractRunInput| {
            ready(spoke_ok(ExtractionResult {
                candidates: Vec::new(),
                method: Some(String::new()),
                coverage_hint: None,
            }))
        },
    ));

    match result {
        SpokeResult::Ok(response) => assert_eq!(
            serde_json::to_value(&response).expect("wire response"),
            json!({ "candidates": [], "run": { "run_id": "run_method_empty" } })
        ),
        other => panic!("expected an extraction success response, got: {other:?}"),
    }
}

#[test]
fn retains_the_shortest_non_empty_method_verbatim() {
    let port = RecordingPort::new(spoke_ok(json!({ "chapter": "text" })));

    let result = pollster::block_on(orchestrate_extract(
        &port,
        make_request(),
        move |_input: ExtractRunInput| {
            ready(spoke_ok(ExtractionResult {
                candidates: Vec::new(),
                method: Some("x".to_owned()),
                coverage_hint: None,
            }))
        },
    ));

    match result {
        SpokeResult::Ok(response) => assert_eq!(
            serde_json::to_value(&response).expect("wire response"),
            json!({ "candidates": [], "run": { "run_id": "run_001", "method": "x" } })
        ),
        other => panic!("expected an extraction success response, got: {other:?}"),
    }
}

#[test]
fn treats_a_null_coverage_hint_as_no_hint() {
    let expected = json!({ "candidates": [], "run": { "run_id": "run_null" } });

    for hint in [None, Some(Value::Null)] {
        let port = RecordingPort::new(spoke_ok(json!({})));

        let result = pollster::block_on(orchestrate_extract(
            &port,
            request_with("run_null"),
            move |_input: ExtractRunInput| {
                ready(spoke_ok(ExtractionResult {
                    candidates: Vec::new(),
                    method: None,
                    coverage_hint: hint,
                }))
            },
        ));

        match result {
            SpokeResult::Ok(response) => assert_eq!(
                serde_json::to_value(&response).expect("wire response"),
                expected
            ),
            other => panic!("expected an extraction success response, got: {other:?}"),
        }
    }
}

#[test]
fn missing_port_double_rejects_capability_port_missing_for_ke_extraction() {
    // `&dyn ExtractionPort` makes absence unrepresentable at the type level;
    // the double returns the same reject the TS runtime guard produces,
    // pinning the capability blame parity.
    struct MissingExtractionPort;

    #[async_trait]
    impl ExtractionPort for MissingExtractionPort {
        async fn load_extraction_input(&self, _request: &ExtractRequest) -> SpokeResult<Value> {
            let mut details = Map::new();
            details.insert("capability".into(), Value::String("ke-extraction".into()));
            spoke_reject(
                SpokeRejectCode::CapabilityPortMissing,
                "no extraction port",
                Some(details),
            )
        }
    }

    let result = pollster::block_on(orchestrate_extract(
        &MissingExtractionPort,
        make_request(),
        move |_input: ExtractRunInput| ready(spoke_ok(result_of(Vec::new()))),
    ));

    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            let details = reject.details.expect("details present");
            assert_eq!(
                details.get("capability").and_then(Value::as_str),
                Some("ke-extraction")
            );
        }
        other => panic!("expected a capability-port-missing reject, got: {other:?}"),
    }
}
