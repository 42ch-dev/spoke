---
module: spoke-connect
date: 2026-08-01
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["implementing step-up or delegated capability auth on a connect session", "designing token-based auth for a connect product", "extending the capability-token method (revocation, refresh)", "porting the capability-token rules to another language"]
tags: [spoke-connect, capability-token, auth, jcs, ed25519, trusted-issuers, offline-validation, step-up]
---

# capability-token auth method (step-up capability grant)

## Context

`capability-token` is the second normative connect auth method (spec §Auth model, §Method — capability-token), implemented in `crates/spoke-connect/src/core/capability_token.rs` with transport wiring in the node (auth exchange protocol `/spoke/connect/auth/1.0.0`, dispatch-gate rework). It is a **step-up / mid-session capability grant** on top of the `noise-peerid` hello identity: a trusted issuer signs a short claim set over RFC 8785 JCS with Ed25519, and the proof rides the `ConnectAuthChallenge` / `ConnectAuthResponse` exchange and optionally the `ConnectInvokeRequest.auth` blob. Validation is **offline** — signature + trusted-issuer list + subject/audience/expiry + capability membership; there is no revocation list, no refresh token, and no issuance endpoint in protocol version 1.

## Guidance

### Claims / proof design (fail closed on shape)

- The signed claims object is **exactly** `{iss, sub, aud, capabilities, exp}` plus optional `iat` / `jti`. `#[serde(deny_unknown_fields)]` on both claims and the wire wrapper: unknown claim keys and unknown wrapper keys **reject** at deserialization, so the JCS bytes stay intentional and a malformed proof never reaches validation.
- The signature covers **only** `JCS(claims)` — not the `{v, claims, sig}` wrapper. Canonicalize with RFC 8785 JCS (`serde_jcs`), sign with the **issuer** Ed25519 private key, encode the raw 64 bytes base64url without padding.
- Issuance binds the key to the claim: the signing key MUST derive `claims.iss` (refused before signing otherwise) — a token cannot be minted by a key other than the issuer it names.
- Wire wrapper: `{ "v": 1, "claims": {…}, "sig": "<base64url-no-pad>" }` — `proof` is an OpaqueJson **object**, not a bare string; `v` must equal the current token version (1).

### Trust root: `trusted_issuers`, empty ⇒ disabled

`ConnectConfig.trusted_issuers: Vec<String>` is the deployment-configured list of issuer `peer_id` strings (parallel to the `noise-peerid` allowlist). **Empty list ⇒ the method is disabled**: no challenges are offered and every proof is rejected (fail closed), even a perfectly valid one. `claims.iss` must be an **exact-string member** of the list after signature verification.

### Subject / audience / expiry checks

| Check | Rule |
|---|---|
| `sub` | Must equal the authenticated session peer's `peer_id` (the peer that passed the `noise-peerid` hello) — tokens are not transferable across peers |
| `aud` | Must equal **this node's** `peer_id` string |
| `exp` | Required; reject when `now >= exp` |
| `iat` | When present, must not be beyond the ±60 s clock-skew window ahead of `now` (`CLOCK_SKEW_SECONDS`); past `iat` is always accepted |
| `jti` | When present, must be non-empty; **not consulted by protocol version 1 validation — reserved for a future revocation design** |

### base58btc inverse peer-id decode

Verification needs the issuer's public key without any key exchange: `ed25519_pubkey_from_peer_id` **recovers it from the `iss` string itself**. Ed25519 peer ids are identity multihashes (protobuf `PublicKey` → multihash `0x00` → base58btc), so the mapping is an encoding inversion — the key is inside the id. Non-Ed25519-shaped `iss` values reject as `TokenInvalid`.

### Capability matching: membership, not equality

For an invoke, the token authorizes the op iff **`required` ∈ `claims.capabilities`** (membership / subset-of-grant). Extra capabilities on the token are ignored when unused; matching is **not** exact-list equality between `claims.capabilities` and `negotiated_capabilities`. The token does **not** replace the session's `negotiated_capabilities`. Dispatch order when the method is in use:

1. Sequence / correlation checks
2. Optional capability-token gate when policy requires a valid token (config or present `auth`)
3. `dispatch_allowed(op, negotiated_capabilities)`
4. Handler

Both the token grant and the negotiated set must allow the op when the token gate is active.

### Challenge / response and per-invoke auth

- The server challenges only when `trusted_issuers` is non-empty **and** `require_capability_token` is true (default `false` — the `noise-peerid`-only behavior is the default). Challenge: `method: "capability-token"`, fresh `challenge_id`, random nonce ≥ 16 chars, bound in a **one-shot pending slot** (anti-replay for the challenge slot).
- The peer answers through its `capability_token_provider` hook (called with the challenger's `peer_id` — the token's `aud`); a missing provider, provider error, or unknown method drops the exchange. A valid response marks the session token-authorized (`capability_token_ok`) and completes the pending `connect`; a rejected/stale response consumes the slot and the session stays unauthorized for invokes — the pending `connect` resolves only at the handshake timeout (fail closed).
- **Session grant lifetime**: the challenge-validated grant is held for the **session lifetime** and is not expiry-rechecked per invoke. Mid-session expiry enforcement is a product concern: implement a re-challenge flow or attach a per-invoke `auth`.
- **Per-invoke `auth`**: `ConnectInvokeRequest.auth` optionally carries the same proof object; when present it is validated on **every** invoke (expiry re-checked; same issuer/subject/audience rules), independent of the challenge flag. With the require-flag active, an invoke without a validated grant is rejected.

### Error vocabulary: open strings, no schema change

Error codes ride the existing open-string `ErrorEnvelope` vocabulary — no new envelope fields:

| Condition | `code` |
|---|---|
| Missing/invalid token when required; signature / issuer / audience / subject / expiry / malformed-proof failure; unknown `method` on an auth response | `auth_failed` |
| Token valid but capability not granted for the requested `op` | `op_unsupported` (same path as dispatch-deny) |

Implementations MAY distinguish failure reasons in `message` / optional `details`; the machine code stays as above.

## Why This Matters

The method delivers delegated, capability-scoped step-up authorization with **zero online infrastructure**: the verifying node needs only the issuer list and the peer-id strings already in the session. Fail-closed is built into the shape (unknown keys reject, empty trust list disables the method, signature checked before trust-list membership), and the issuer-key-from-peer-id inversion keeps verification self-contained. The membership rule means issuers can grant supersets without breaking callers, and `jti` leaves a hook for revocation without protocol churn.

## When to Apply

- Adding token-based step-up auth to a connect product — configure `trusted_issuers` (+ `require_capability_token` / `capability_token_provider`) and reuse the pure core rules; do not invent a parallel validation path.
- Porting the capability-token rules to another language (Path A or a uniffi target) — port the claim set, JCS sign/verify, trust-root, subject/audience/expiry, and membership rules against the spec; the `TokenInvalid` failure variant already crosses the Swift FFI enums.
- Future revocation / refresh work — `jti` is the reserved hook; the offline model means revocation is a later design decision, not a v1 gap.

## Examples

### Wire proof (OpaqueJson object)

```json
{
  "v": 1,
  "claims": {
    "iss": "12D3KooW…",
    "sub": "12D3KooW…",
    "aud": "12D3KooW…",
    "capabilities": ["spoke-baseline"],
    "exp": 1790000000
  },
  "sig": "<base64url-no-pad Ed25519 over JCS(claims)>"
}
```

### Validation order (pure core `verify_capability_token`)

1. `v` is the current token version
2. Signature verifies over `JCS(claims)` with the public key recovered from `iss`
3. `iss` ∈ `trusted_issuers` (empty list ⇒ reject every proof)
4. `sub` == authenticated session peer
5. `aud` == this node's `peer_id`
6. `exp` required; `now >= exp` rejects
7. `iat` within the ±60 s skew window ahead of `now`
8. `jti` non-empty when present
9. Return `claims.capabilities` as the grant for the dispatch gate

## See also

- [`spoke-connect-wire-and-auth.md`](spoke-connect-wire-and-auth.md) — the auth model and `method` vocabulary this method extends; the error-envelope reuse rule.
- [`connect-session-core-ffi-boundary.md`](connect-session-core-ffi-boundary.md) — the pure-core extraction; `TokenInvalid` maps into the exported Swift `CoreError` enum.
- [`connect-ts-client-sdk.md`](connect-ts-client-sdk.md) — the TS Path A port, whose first slice does not implement capability-token (normative, deferred).
- [`spoke-connect.md`](../../specs/spoke-connect.md) §Method — capability-token — the normative claim set, trust root, and validation rules.
