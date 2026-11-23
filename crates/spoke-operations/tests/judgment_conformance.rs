//! Judgment conformance corpus runner (spoke-judgment wire contract).
//!
//! Executes `fixtures/judgment-conformance/`: every `valid.json` envelope
//! must pass `validate_judgment`, every `refusals.json` case must reject
//! `INVALID_INPUT` with `details.field` equal to the case's exact logical
//! path, and every `canonical-parity.json` pair must canonicalize to the
//! case's expected RFC 8785 bytes on the raw wire value for both members;
//! a typed `Judgment` round-trip is additionally asserted for the left
//! member only, and only when the case's `typed_parity` flag is true (it is
//! compared against the raw left bytes, not a separate expected value).
//! The TS runner
//! (`packages/spoke-operations/tests/judgment-conformance.test.ts`) asserts
//! the same corpus, so shared `expected_canonical` strings make each vector
//! a cross-stack byte-parity check.
//!
//! Valid cases may carry `proto_members`, which only the TypeScript runner
//! grafts onto a prototype chain — JSON cannot express inherited members and
//! this runner asserts the wire JSON, so the field is ignored here.
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
fn canonical_parity_cases_produce_expected_bytes() {
    let scenario = load_scenario("canonical-parity.json");
    for case in cases(&scenario) {
        let id = case["id"].as_str().expect("case id");
        let left = &case["left"];
        let right = &case["right"];
        // `equal: false` cases carry per-side expected bytes under
        // `expected_canonical.left` / `.right`; equal cases carry one shared
        // string.
        let equal = case["equal"].as_bool().unwrap_or(true);
        let expected = &case["expected_canonical"];
        let expected_left = if equal {
            expected.as_str().expect("expected_canonical")
        } else {
            expected["left"].as_str().expect("expected_canonical.left")
        };
        let expected_right = if equal {
            expected.as_str().expect("expected_canonical")
        } else {
            expected["right"].as_str().expect("expected_canonical.right")
        };

        // Both producers must pass the wire-shape gate before byte claims.
        assert!(
            matches!(validate_judgment(left), SpokeResult::Ok(())),
            "case {id}: left must validate"
        );
        assert!(
            matches!(validate_judgment(right), SpokeResult::Ok(())),
            "case {id}: right must validate"
        );

        // Raw wire-value canonical bytes equal the fixture's expected bytes,
        // which the TS runner asserts against `canonicalize` output.
        let left_bytes = serde_jcs::to_vec(left).expect("left canonicalizes");
        let right_bytes = serde_jcs::to_vec(right).expect("right canonicalizes");
        assert_eq!(
            String::from_utf8(left_bytes.clone()).expect("left bytes are UTF-8"),
            expected_left,
            "case {id}: left canonical bytes"
        );
        assert_eq!(
            String::from_utf8(right_bytes.clone()).expect("right bytes are UTF-8"),
            expected_right,
            "case {id}: right canonical bytes"
        );

        // The comparison helper reports the contract's wire-bytes equality.
        assert!(
            matches!(
                judgments_equal_canonical(left, right),
                SpokeResult::Ok(value) if value == equal
            ),
            "case {id}: judgments_equal_canonical"
        );

        // Typed round-trip is only asserted for cases whose members survive
        // typed serialization unchanged; `typed_parity: false` marks cases
        // where the generated type normalizes wire content (empty `detail`
        // maps, non-UTC `created_at` spellings).
        if case["typed_parity"].as_bool().unwrap_or(true) {
            let left_typed = typed(left);
            let typed_bytes =
                serde_jcs::to_vec(&left_typed).expect("typed canonicalizes");
            assert_eq!(
                typed_bytes, left_bytes,
                "case {id}: typed serialization preserves canonical bytes"
            );
        }
    }
}
