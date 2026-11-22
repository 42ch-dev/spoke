# ADR: Ship judgment as a manifest-only data capability with a dedicated reference and wire-value validation
Status: proposed — recorded against the shipped judgment surface; promotion to `implemented/` follows PM acceptance of the `knowledge-evolution-p3-judgment` plan
Date: 2026-11-23
Scope: SPOKE judgment wire contract (`schemas/data/judgment.schema.json`), operations-library validation helpers, capability advertisement
Spec: `.agents/specs/protocol/spoke-judgment.md`

## Problem

Products need to exchange assessment claims — a consistency verdict on a KnowledgeEntry, an evidentiary-support verdict, an acceptance decision, a review outcome — without agreeing on an assessment engine. Without a settled envelope each product invents its own DTO and judgments cannot cross producers or be compared byte-for-byte.

Four decisions had no other home:

1. The reference shape. `CognitionReference` / `CognitionTarget` already exist, but they carry cognition-scoped semantics: mandatory namespace, target and assessment-context roles. A judgment reference is a lighter thing — kind, id, optional product scope, optional revision — and reusing the cognition carriers would force judgment references to carry fields they do not mean.
2. The verdict shape. A fully open verdict gives no portable outcome; a fully closed one has nowhere to put product assessment content.
3. The capability form. If `spoke-judgment` were a Connect operation it would need a dispatch row, request/response envelopes, and protocol-level behavior — but judgment is data plus a conformance obligation, not a protocol action.
4. The validation input. The existing wire schemas decode into generated typed structs. Validating a decoded struct reports violations as opaque decode errors (`additionalProperties: false` violations, type mismatches) with no stable field path, and reserved-member locations inside `verdict.detail` no longer exist as data once decoded.

## Proposal

1. **Dedicated `JudgmentRef`.** A closed four-field reference (`kind`, `id`, optional `namespace`, conditional positive `revision`) used by `Judgment.target` and every `Judgment.evidence[]` item. It reuses nothing from the cognition family.
2. **`verdict` as closed core plus open detail.** Required non-empty `outcome`, optional `confidence` in `[0, 1]`, optional product `detail` object whose direct members must not duplicate protocol envelope locations (reserved names: `target`, `evidence`, `judgment_kind`, `outcome`).
3. **`spoke-judgment` as a manifest-only data/conformance capability.** A host that validates or produces `Judgment` advertises the flag in `HostCapabilityManifest.capabilities`; the capability defines no Connect dispatch row and no `op`. Tools whose payloads carry `Judgment` are declared via `ToolDescriptor` and gated on exact membership of their own `tools.<ns>.<tool_id>` string in `negotiated_capabilities`, exactly as before.
4. **Evidence revision pinning.** Every evidence item is a `JudgmentEvidenceRef` — a `JudgmentRef` narrowed to a required positive `revision` — so evidence always names the exact revision relied on. The target's `revision` stays optional.
5. **`validateJudgment` on the undecoded wire value.** Rust takes `&serde_json::Value`; TypeScript takes the generated structural `Judgment` type, which at runtime is the same undecoded value. Every shape violation rejects `INVALID_INPUT` with `details.field` carrying the exact logical field path (dotted members, bracketed array indices). A sibling helper `judgmentsEqualCanonical` / `judgments_equal_canonical` compares RFC 8785 canonical bytes and covers the complete envelope including `verdict.detail`, `extensions`, and present empty arrays and objects.

## Alternatives considered

- **Reuse `CognitionReference` / `CognitionTarget` for judgment references.** One fewer reference type, but judgment references inherit mandatory namespace and assessment-context semantics they do not carry, and cognition's carrier evolution would start breaking judgment producers. A dedicated four-field reference keeps both envelopes minimal and independently evolvable.
- **Fully open or fully closed `verdict`.** An open verdict offers no portable outcome or confidence for cross-product reading; a closed one forces product assessment content into `extensions` where it loses its verdict-scoped home and the reserved-location protection. The core-plus-detail split gives portability where it is real and containment where it is needed.
- **No reserved-member check on `verdict.detail`.** Simpler validator, but `verdict.detail.target` and `verdict.detail.evidence` would become second authority sites for protocol locations, splitting readers between two places for the same fact. The check protects protocol field locations only; it deliberately does not validate product `detail` shape.
- **Make `spoke-judgment` a Connect op with its own dispatch row.** That invents protocol behavior and envelopes for something that is data. Existing `tools.*` transport already carries arbitrary payloads behind tool-id gating; a data/validator capability flag advertises conformance without touching dispatch.
- **Gate `tools.*` operations that carry `Judgment` on `spoke-judgment`.** This stacks a second, fuzzy authorization source on top of exact tool-id membership. The shipped rule keeps one gate: the full operation string.
- **Pin the target revision as well.** Judgments on live or drifting targets are legitimate (a product may assess "the current thing"); evidence is different — the exact revisions a judgment relies on must be named for the claim to be re-interpretable. The asymmetry is deliberate.
- **Validate a decoded typed struct (or run a JSON Schema validator at the gate).** Decoding first turns contract refusals into opaque errors without stable field paths — and reserved-member violations cannot be reported at all once `detail` is decoded into product shape. A schema validator at the boundary does not produce the shared `SpokeResult` / `SpokeRejectCode` reject envelope the operations library guarantees in both languages. Hand-written helpers over the wire value keep the refusal vocabulary (`INVALID_INPUT` + `details.field`) deterministic and language-identical.

## Acceptance criteria

- `schemas/data/judgment.schema.json` defines `Judgment`, `JudgmentRef`, `JudgmentEvidenceRef`, `Verdict` with the field set above; both type packages are generated from it.
- `@42ch/spoke-operations` exports `validateJudgment(judgment: Judgment): SpokeResult<void>` and `judgmentsEqualCanonical(left, right): SpokeResult<boolean>`; `spoke-operations` exports the behaviorally identical snake_case `validate_judgment(&Value)` and `judgments_equal_canonical(&Judgment, &Judgment)`.
- Every shape violation on either side rejects `INVALID_INPUT` with `details.field` set to the logical field path; conformance suites on both sides cover the closed-envelope violations, reserved `detail` members, empty `evidence`, and canonical equality/inequality.
- No Connect dispatch row, `op`, or envelope exists for `spoke-judgment`; `tools.*` gating is unchanged and tests tool-id membership only.

## Risks

- The reserved `detail` member list is a minimal fixed set; products wanting a same-named concept inside `detail` must rename. Cost: a naming constraint, in exchange for one authority site per protocol location.
- Open vocabularies (`judgment_kind`, `verdict.outcome`) mean the protocol guarantees envelope interoperability, not semantic interoperability; readers must know the producer's vocabulary. The four core `judgment_kind` values mitigate, not remove, this.
- RFC 8785 byte equality depends on both canonicalization stacks (`serde_jcs`, `canonicalize`) agreeing on edge numbers; lockstep parity suites bear this risk.
- The Rust `&Value` and TS `Judgment` signatures are asymmetric. The TS type is structural and erased at runtime, so a caller can pass a value whose runtime shape disagrees; the validator therefore fully re-checks shape at runtime in both languages rather than trusting the parameter type. Behavior is identical; the signatures document different trust levels.
