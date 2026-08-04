---
module: spoke-connect
date: 2026-08-04
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
applies_when: ["porting capability-token verify to a new language binding", "decoding base64url signatures across host runtimes", "evaluating whether a base64 decoder swap is safe", "adding new session-core parity invariants between TS and Rust"]
tags: [spoke-connect, capability-token, base64, canonical-encoding, ts-rust-parity, defense-in-depth, rfc-4648]
---

# base64url canonical-signature round-trip across TS and Rust

## Context

The capability-token `sig` field is the base64url-no-padding encoding of the 64 raw Ed25519 signature bytes over `JCS(claims)`. RFC 4648 base64url has a **canonical encoding** for any byte sequence: a 64-byte input always produces the same 86-character string. Non-canonical encodings exist for the final character's slack bits (when the input length is not a multiple of 3, the last base64 character carries data bits + slack bits that should be zero), but those non-canonical forms decode to the same bytes under a **lenient** decoder.

The TS client (`packages/spoke-connect-ts`) and the Rust reference (`crates/spoke-connect`) both verify capability tokens. They use different base64 decoders:

- **TS** uses an `atob`-derived decoder that is **slack-lenient** — it accepts non-zero slack bits in the final character and decodes them to the same bytes as the canonical form.
- **Rust** uses `base64` 0.22's `URL_SAFE_NO_PAD` engine, which is **strict** (`decode_allow_trailing_bits: false`) — non-zero slack bits are rejected at decode with `InvalidLastSymbol`.

This asymmetry means a parity invariant that is **load-bearing on the TS side** is **unreachable on the Rust side** under the current decoder. The invariant in question: the canonical-sig round-trip check (`encode(decode(sig)) === sig`) that rejects any non-canonical encoding of the signature bytes.

## Guidance

### Keep the round-trip check on both sides, even when one side makes it unreachable

The TS verify path runs:

```ts
const raw = base64UrlDecode(proof.sig);
if (base64UrlEncode(raw) !== proof.sig) {
  throw new CoreError("token_invalid", "signature is not canonical base64url (no padding)");
}
```

The Rust verify path runs the structural equivalent:

```rust
let raw = URL_SAFE_NO_PAD
    .decode(proof.sig.as_str())
    .map_err(|_| CoreError::TokenInvalid("signature is not valid base64url".into()))?;
if URL_SAFE_NO_PAD.encode(&raw) != proof.sig {
    return Err(CoreError::TokenInvalid(
        "signature is not canonical base64url (no padding)".into(),
    ));
}
```

With `base64` 0.22 strict decoding, the second branch is **dead code today** — any input that would trip the round-trip check is already rejected by `URL_SAFE_NO_PAD.decode`. Keep the branch anyway. Two reasons:

1. **Source-level TS↔Rust parity invariant.** The round-trip check is the explicit, portable statement of "canonical encoding only." A future contributor reading either side sees the same rule, the same error variant, and the same defense intent. Removing the Rust branch because it is unreachable today would create an asymmetric surface that looks like Rust permits non-canonical encodings — misleading on a security path.
2. **Defense in depth against decoder relaxation.** If a future base64 upgrade, feature flag, or engine swap relaxes the decoder (`decode_allow_trailing_bits: true`), the round-trip check resumes being load-bearing without any other code change. The branch is a one-line insurance premium; deleting it saves nothing meaningful.

The normative rule belongs in the protocol spec (`.mstar/specs/spoke-connect.md` §Method — capability-token: "Canonical encoding (normative): the `sig` field MUST be the unique RFC 4648 canonical base64url encoding"). The implementation check on each side is the machine enforcement of that rule.

### When porting to a new language, verify the local decoder's strictness first

A new language binding (Go, Python, Swift, Kotlin, C#) implementing `verify_capability_token` should determine its local base64url decoder's slack-bit behavior before deciding whether to include the round-trip check:

- **Strict decoder** (rejects non-zero slack bits at decode, like Rust 0.22): include the round-trip check anyway for source-level parity with TS and Rust. Document the unreachability in a comment so the next reader understands it is defense in depth.
- **Lenient decoder** (accepts non-zero slack bits, like TS `atob`): the round-trip check is **load-bearing** — omitting it admits non-canonical signatures, breaking the canonical-encoding normative rule.

Do not assume a language's standard library matches either behavior; verify by writing a scratch repro that decodes a known non-canonical encoding (mutate the final character's slack bits, preserve the low data bits) and observe whether decode succeeds.

### Cross-language test: pin the rejection set, not the code path

The non-canonical-sig test (`non_canonical_signature_rejected`) should assert that `verify_capability_token` rejects the mutated input with the `TokenInvalid` family — **not** that it rejects via the round-trip branch specifically. That keeps the test green across decoder-strictness changes:

- On TS (lenient `atob`), decode succeeds and the round-trip branch fires → `TokenInvalid`.
- On Rust 0.22 (strict), decode itself rejects → `TokenInvalid`.
- On a hypothetical future Rust with relaxed decoder, the round-trip branch would resume firing → `TokenInvalid`.

Same observable outcome (`TokenInvalid`), same input, different internal path. The test pins the contract; the implementation owns the path.

### Slack-bit mutation formula for the test fixture

When constructing the non-canonical input, mutate only the slack bits of the final base64 character — preserve the low 2 data bits so the mutated string still represents the same 64 bytes under a lenient decoder (otherwise the test would be exercising a different signature, not a non-canonical encoding of the same one).

A robust formula over the base64 alphabet: take the final character's index, extract `data = idx & 0b11` (low 2 bits) and `slack = idx >> 2` (high 4 bits), compute `slack_new = (slack % 15) + 1` (always 1..=15, never wraps to canonical 0, never equals the original slack), and reassemble `idx_new = data | (slack_new << 2)`. The wrap-resistant modulo-15 (not 16) is what makes the mutation safe across arbitrary source signatures: the original sig's final slack value does not affect whether the mutation produces a valid non-canonical sibling.

## Why this matters

The TS↔Rust session-core parity contract (root `AGENTS.md`, `.mstar/specs/spoke-connect-ts-route.md` §Session-core parity) scopes to **rules**, not implementations. "Canonical base64url signatures only" is a rule; both sides must enforce it. The implementation path each side takes (load-bearing round-trip on TS, defense-in-depth round-trip + strict-decode on Rust) is a local concern — invisible to integrators, who only see that a non-canonical sig is rejected identically on both sides.

Treating the unreachability as a reason to delete the Rust branch would silently narrow the parity surface from "rule" to "happens to reject because of decoder version," coupling the contract to a transitive dependency. Future decoder upgrades would then need a parity audit; with the round-trip check kept, they need only a test-suite run.

## When to apply

- **Porting capability-token verify to a new language.** Verify decoder strictness, then include the round-trip check unconditionally.
- **Considering a base64 engine swap on either side.** Re-run the cross-language golden vector + the non-canonical-sig test; the test pins the observable contract.
- **Auditing session-core parity surface.** Treat the round-trip check as part of the rule even if locally unreachable; document the unreachability in the code comment so reviewers do not flag it as dead code to remove.
- **Writing the normative spec rule.** Phrase the rule affirmatively ("MUST be the unique canonical RFC 4648 encoding; receivers MUST reject any `sig` that does not round-trip through `decode → encode` equality") — the rule is decoder-independent; the implementation owns the strictness.

## Examples

### TS verify path (load-bearing round-trip)

`packages/spoke-connect-ts/src/core/capability-token.ts`:

```ts
let signature: Uint8Array;
try {
  signature = base64UrlDecode(proof.sig);
} catch {
  throw new CoreError("token_invalid", "signature is not valid base64url");
}
if (base64UrlEncode(signature) !== proof.sig) {
  throw new CoreError("token_invalid", "signature is not canonical base64url (no padding)");
}
if (signature.length !== 64) {
  throw new CoreError("token_invalid", "signature is not 64 bytes");
}
```

`base64UrlDecode` is slack-lenient (derived from `atob`), so a non-canonical encoding decodes successfully and the round-trip check fires.

### Rust verify path (defense-in-depth round-trip under strict decoder)

`crates/spoke-connect/src/core/capability_token.rs`:

```rust
let raw = URL_SAFE_NO_PAD
    .decode(proof.sig.as_str())
    .map_err(|_| CoreError::TokenInvalid("signature is not valid base64url".into()))?;
// Canonical base64url (RFC 4648) round-trip check. `URL_SAFE_NO_PAD` strict
// config already rejects non-zero slack bits at decode, so this branch is
// defense-in-depth + the TS↔Rust parity invariant (the check is load-bearing
// on the TS side where the decoder is slack-lenient).
if URL_SAFE_NO_PAD.encode(&raw) != proof.sig {
    return Err(CoreError::TokenInvalid(
        "signature is not canonical base64url (no padding)".into(),
    ));
}
let signature = Signature::from_slice(&raw)
    .map_err(|_| CoreError::TokenInvalid("signature is not 64 bytes".into()))?;
```

### Non-canonical-sig test (pins observable rejection)

```rust
#[test]
fn non_canonical_signature_rejected() {
    let now = 1_000_000_000;
    let (mut proof, trusted, peers) = happy_token(now, &["spoke-baseline"]);
    // Flip the slack bits of the final base64 character; preserve the low 2
    // data bits so the mutated string still encodes the same 64 bytes under
    // a slack-lenient decoder.
    let last_idx = BASE64_URL
        .iter()
        .position(|&c| c as char == proof.sig.chars().last().unwrap())
        .unwrap();
    let data = last_idx & 0b11;
    let slack = last_idx >> 2;
    let slack_new = (slack % 15) + 1;
    let mutated_idx = data | (slack_new << 2);
    proof.sig.pop();
    proof.sig.push(BASE64_URL[mutated_idx] as char);
    let err = verify_capability_token(&proof, &trusted, &peers[1], &peers[0], now)
        .expect_err("non-canonical sig");
    assert!(matches!(err, CoreError::TokenInvalid(_)));
}
```

The test does not assert which branch rejected — only that the `TokenInvalid` family fires. On Rust 0.22, the strict `decode` rejects before the round-trip branch; the assertion still holds.

## See also

- [`connect-capability-token-auth.md`](connect-capability-token-auth.md) — the capability-token auth method this round-trip check protects.
- [`connect-session-core-ffi-boundary.md`](connect-session-core-ffi-boundary.md) — the session-core extraction and TS↔Rust parity surface (4 shared rules + helper boundary).
- [`connect-identity-parity-proof.md`](../testing-patterns/connect-identity-parity-proof.md) — cross-language byte-parity methodology for identity / signature reproducibility.
- `.mstar/specs/spoke-connect.md` §Method — capability-token — "Canonical encoding (normative)" rule.
- `.mstar/specs/spoke-connect-ts-route.md` §Session-core parity — the parity contract this check belongs to.
