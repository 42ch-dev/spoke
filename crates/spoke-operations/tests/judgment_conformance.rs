//! Judgment conformance corpus runner (spoke-judgment wire contract).
//!
//! Executes `fixtures/judgment-conformance/`: every `valid.json` envelope
//! must pass `validate_judgment`, every `refusals.json` case must reject
//! `INVALID_INPUT` with `details.field` equal to the case's exact logical
//! path, and every `canonical-parity.json` pair must canonicalize to the
//! case's expected RFC 8785 bytes — on the raw wire value and on the typed
//! `Judgment` alike. The TS runner
//! (`packages/spoke-operations/tests/judgment-conformance.test.ts`) asserts
//! the same corpus, so shared `expected_canonical` strings make each vector
//! a cross-stack byte-parity check.

use serde_json::Value;
use spoke_operations::{
    judgments_equal_canonical, validate_judgment, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::judgment::Judgment;
use std::path::{Path, PathBuf};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/judgment-conformance")
}

fn load_scenario(filename: &str) -> Value {
    let path = fixture_root().join(filename);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("failed to parse fixture {}: {error}", path.display()))
}

fn cases(scenario: &Value) -> &[Value] {
    scenario["cases"]
        .as_array()
        .expect("scenario carries a cases array")
}

fn typed(judgment: &Value) -> Judgment {
    serde_json::from_value(judgment.clone()).expect("fixture parses into Judgment")
}

fn expect_invalid(result: SpokeResult<()>, field: &str, case_id: &str) {
    let SpokeResult::Reject(reject) = result else {
        panic!("case {case_id}: expected INVALID_INPUT reject for {field}");
    };
    assert_eq!(
        reject.code,
        SpokeRejectCode::InvalidInput,
        "case {case_id}: reject code"
    );
    let details = reject.details.expect("case carries details");
    assert_eq!(
        details.get("field").and_then(Value::as_str),
        Some(field),
        "case {case_id}: details.field"
    );
}

#[test]
fn valid_cases_pass_validate_judgment() {
    let scenario = load_scenario("valid.json");
    for case in cases(&scenario) {
        let id = case["id"].as_str().expect("case id");
        let judgment = &case["judgment"];
        assert!(
            matches!(validate_judgment(judgment), SpokeResult::Ok(())),
            "case {id}: valid envelope must pass validate_judgment"
        );
        // Every valid fixture must also decode into the typed wire type —
        // the corpus stays inside the generated schema's envelope.
        typed(judgment);
    }
}

#[test]
fn refusal_cases_reject_invalid_input_with_field() {
    let scenario = load_scenario("refusals.json");
    for case in cases(&scenario) {
        let id = case["id"].as_str().expect("case id");
        let field = case["expect"]["field"].as_str().expect("expect.field");
        expect_invalid(validate_judgment(&case["judgment"]), field, id);
    }
}

#[test]
fn canonical_parity_cases_produce_identical_bytes() {
    let scenario = load_scenario("canonical-parity.json");
    for case in cases(&scenario) {
        let id = case["id"].as_str().expect("case id");
        let left = &case["left"];
        let right = &case["right"];
        let expected = case["expected_canonical"]
            .as_str()
            .expect("expected_canonical");

        // Both producers must pass the wire-shape gate before byte claims.
        assert!(
            matches!(validate_judgment(left), SpokeResult::Ok(())),
            "case {id}: left must validate"
        );
        assert!(
            matches!(validate_judgment(right), SpokeResult::Ok(())),
            "case {id}: right must validate"
        );

        // Wire-value canonical bytes equal the fixture's expected bytes,
        // which the TS runner asserts against `canonicalize` output.
        let left_bytes = serde_jcs::to_vec(left).expect("left canonicalizes");
        let right_bytes = serde_jcs::to_vec(right).expect("right canonicalizes");
        assert_eq!(
            String::from_utf8(left_bytes.clone()).expect("left bytes are UTF-8"),
            expected,
            "case {id}: left canonical bytes"
        );
        assert_eq!(
            String::from_utf8(right_bytes.clone()).expect("right bytes are UTF-8"),
            expected,
            "case {id}: right canonical bytes"
        );

        // The typed helper reports byte-equality and the typed
        // serialization reproduces the same canonical bytes.
        let left_typed = typed(left);
        let right_typed = typed(right);
        assert!(
            matches!(
                judgments_equal_canonical(&left_typed, &right_typed),
                SpokeResult::Ok(true)
            ),
            "case {id}: judgments_equal_canonical"
        );
        let typed_bytes = serde_jcs::to_vec(&left_typed).expect("typed canonicalizes");
        assert_eq!(
            typed_bytes, left_bytes,
            "case {id}: typed serialization preserves canonical bytes"
        );
    }
}
