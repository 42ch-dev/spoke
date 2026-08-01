---
module: spoke-connect
date: 2026-07-31
last_updated: 2026-08-01
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["adding a transport/interaction envelope family", "implementing a spoke-connect adapter or SDK", "designing network auth over libp2p for SPOKE", "porting connect identity or session rules to a new language"]
tags: [spoke-connect, connect-envelope, jcs-signature, peer-id-derivation, embedding-model, dispatch-gate, session-ordering, noise-peerid]
---

# spoke-connect wire family and authenticated interaction model

## Context

SPOKE standardizes the KnowledgeEntry data wire and transport-agnostic ops wire, but the protocol explicitly excludes transport bindings and network discovery from baseline. Multi-adapter client interaction over a shared network needs a standard interaction layer: discovery, handshake, authentication, session-ordered invocation. The `spoke-connect` capability family adds this as an opt-in fourth wire family (`schemas/connect/`) beside `data/`, `ops/`, and `common/`, so client adapters get a protocol-grade contract instead of reinventing transport, auth, and ordering per product.

## Guidance

The connect family is **six closed envelopes** under `schemas/connect/`, all carrying required `extensions` (ExtensionMap) and reusing existing shared definitions by `$ref` (no parallel identity/error systems):

| Envelope | Role |
|----------|------|
| `ConnectHello` | Signed manifest exchange - `protocol_version`, `peer_id`, `nonce`, embedded `HostCapabilityManifest` (`$ref`), `signature` |
| `ConnectSession` | Session context - peer pair, `opened_at` (Timestamp), negotiated `capabilities[]`, `initial_sequence` (const 0) |
| `ConnectInvokeRequest` | Remote op call - `sequence` (0..2^53-1 JSON-safe), open-string `op`, opaque `payload` (OpaqueJson wrapping an existing ops request envelope), optional `auth` |
| `ConnectInvokeResponse` | Remote op reply - oneOf success (`payload` OpaqueJson) \| error (`$ref` ErrorEnvelope); echoes `session_id`/`sequence`/`request_id` |
| `ConnectAuthChallenge` / `ConnectAuthResponse` | Extensible auth - open-string `method` + opaque `proof`; core method `noise-peerid` |

**Signature canonicalization** (`spoke-connect-hello-jcs-v1`): RFC 8785 JSON Canonicalization Scheme over exactly `{protocol_version, peer_id, nonce, host}` - **excluding** top-level `extensions` and `signature`. Receivers MUST NOT use unsigned hello `extensions` for authorization or trust decisions; only the signed fields plus the embedded `host` manifest participate in `noise-peerid` accept/reject.

**Dual identity**: `peer_id` (network trust root derived per the identity-binding formula below - allowlist key) is distinct from `host_id` (manifest application label); they need not be equal. Session `initiator_peer_id`/`responder_peer_id` MUST equal the authenticated hello `peer_id`s; receivers reject mismatching session snapshots.

**Ordering** has two layers: message-level (per-session monotonic `sequence` from 0 per direction, no wrap - exhaustion closes session; `request_id` correlation/echo) and orchestration-level (manifest `roles`-driven peer selection and typical invocation order; explicitly not global consensus).

**Auth model**: `noise-peerid` - libp2p Noise gives transport encryption + peer identity (PeerId from keypair); the protocol layer adds a PeerId allowlist trust root and a signed hello. The auth envelope's open `method` string reserves extension points (`capability-token`, `did`) without implementing them; `proof` is opaque per method.

**Capability flag**: `spoke-connect` is an optional flag in `spoke-protocol-layers.md` (same opt-in pattern as `l2-computable` / `l5-fork` / `narrative-modules`); baseline MUST NOT require it. The baseline exclusions ("Transport bindings", "Network discovery") point at this opt-in family without dropping the exclusion semantics.

**Identity binding (protocol_version 1)** - a normative derivation every connect implementation shares, in any language: `peer_id` is the libp2p PeerId string of the Ed25519 public key. (1) Encode the public key as the libp2p protobuf `PublicKey` message - field 1 `Type` = 1 (Ed25519), field 2 `Data` = the 32 raw bytes. (2) Build the **identity** multihash (code `0x00`) over those protobuf bytes: the digest is the protobuf bytes themselves - **not** a sha2-256 hash of them, and **not** the raw 32-byte key alone. (3) Encode with **base58btc** - no multibase prefix character (no leading `z`), no CIDv1 wrapping. The result equals rust-libp2p `PeerId::to_string()`; every Ed25519-derived peer id shares the `12D3KooW` prefix. Binding rules: receivers authorize on `peer_id` + verified signature (`host_id` stays advisory); when the transport supplies an authenticated peer identity (Noise remote id), the claimed hello `peer_id` MUST equal it and the verify key MUST derive that same `peer_id`; Path A stacks without Noise still MUST derive per the formula and verify the signature against the key that derives the claimed `peer_id`.

**Embedding model** - connect is an embeddable library contract with three layers: **wire contract** (`schemas/connect/` + spec: field tables, JCS, identity binding, framing unit, vocabularies - language-independent protocol SSOT), **session core** (pure rules: hello accept/reject gates, nonce single-use, session state, per-direction sequence allocation and check, `request_id` correlation, allowlist evaluation, op dispatch gate - portable without a shared native library), and **transport adapter** (byte-stream or RPC channel, authenticated transport identity when present, stream lifecycle, dialing, peer metadata, delimiter choice - per language and per product). Two embedding paths: **Path A language-direct** (implement wire + session-core rules in the host language, pair with that language's network stack - no Rust runtime) and **Path B shared core bindings** (export a session-core implementation, e.g. Rust via uniffi, into the host language; transport adapter stays in the host language or the shared core). Both paths MUST produce the same signed hello bytes, the same `peer_id` derivation, and the same accept/reject outcomes for a given input; the reference stack in `crates/spoke-connect` is one Path B-oriented transport demonstration - evidence, not the definition of the rules.

**Op dispatch gate (MUST)** - before executing or forwarding a `ConnectInvokeRequest`, a host that performs op dispatch MUST ensure the capability required by `op` is present in the session's `negotiated_capabilities`; if absent it MUST NOT run the op handler and MUST answer the `ConnectInvokeResponse` error branch (e.g. code `op_unsupported` or `capability_missing`) with no side effects. Core-op capability mapping (protocol_version 1): `upsert`/`promote`/`relate`/`check`/`assemble` → `spoke-baseline` (or a deployment-documented synonym both peers list); `project`/`compute` → `l2-computable`. The gate is evaluated against `negotiated_capabilities` - not the remote manifest alone, not unsigned hello `extensions`. Sequence/correlation checks apply before or with the gate; a failed sequence check also produces no handler side effect.

**Ops-family registration** - a new or modified ops family intended to be remotely invokable over connect MUST: (1) define stable `op` string(s) and keep them listed in the spec's `op` core vocabulary or product docs; (2) keep them in sync with the JSON Schema `description` fields under `schemas/ops/`; (3) declare the capability name(s) hosts must advertise in `HostCapabilityManifest.capabilities` for negotiability; (4) rely on opaque `payload` wrapping of existing ops request/response envelopes - MUST NOT require connect envelope shape changes for new ops fields. Connect schemas and the six envelope shapes stay closed; extensibility is vocabulary + capabilities + opaque payload.

## Why This Matters

Without a standard interaction layer, every multi-adapter product reinvents handshake, auth, ordering, and remote-op envelopes - breaking cross-product interoperability that is SPOKE's north star. Locking the envelopes as wire SSOT with codegen (TS + Rust) means client adapters implement against a stable contract, and the signed-hello + allowlist model gives a defensible auth baseline that later methods (tokens, DID) extend without re-litigating the transport layer. Keeping it opt-in preserves the "protocol repo, not a runtime" boundary: the wire + normative spec live here; the embeddable SDK and product bindings ship separately.

## When to Apply

- Adding any new connect-family envelope or field - follow the closed-envelope + required-`extensions` + `$ref`-reuse rules; bump `protocol_version` for breaking shape changes (negotiated via hello).
- Implementing a spoke-connect adapter or SDK in any language - the JCS signed-field set, identity-binding formula, dual-identity rule, session peer-binding, and sequence semantics are normative; validate `const 0` / sequence range at the wire boundary (generated Rust types do not enforce `const`).
- Designing network auth over libp2p for SPOKE - start from `noise-peerid` + allowlist; gate at `ConnectionEstablished` (before disclosure); bind the verify key to the noise PeerId, not to self-reported hello fields.
- Porting the session core or hello signing to a new language (Path A or Path B) - derive `peer_id` per the identity-binding formula and canonicalize the signed object per the JCS rules; assert against the shared golden vectors (seed 1..=32 → `12D3KooWJ1T…`) so all language paths agree byte-for-byte.
- Extending the ops surface over connect - register new families via the ops-family registration contract (capability + `op` vocabulary + opaque payload); never add connect envelope fields for new ops.

## Examples

### Closed envelope with `$ref` reuse (ConnectHello excerpt)

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["protocol_version", "peer_id", "nonce", "host", "signature", "extensions"],
  "properties": {
    "host": { "$ref": "../data/host-capability-manifest.schema.json" },
    "extensions": { "$ref": "../common/common.schema.json#/definitions/ExtensionMap" }
  }
}
```

### Invoke response oneOf (no parallel status enum)

```json
{ "oneOf": [
  { "required": ["session_id","sequence","request_id","payload","extensions"] },
  { "required": ["session_id","sequence","request_id","error","extensions"],
    "properties": { "error": { "$ref": "../common/error-envelope.schema.json" } } }
]}
```

### peer_id byte layout (identity binding, protocol_version 1)

```text
pubkey (32 bytes)
  → protobuf PublicKey (Ed25519): 0x08 0x01 0x12 0x20 || pubkey   (36 bytes)
  → identity multihash:           0x00 0x24 || pk_bytes          (38 bytes)
  → base58btc (no multibase prefix, no CIDv1):
      "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf"
```

## See also

- [`connect-session-core-ffi-boundary.md`](connect-session-core-ffi-boundary.md) - the pure session-core extraction and sync/async FFI facade that implements these rules for Path B.
- [`connect-identity-parity-proof.md`](../testing-patterns/connect-identity-parity-proof.md) - the cross-language identity-byte parity methodology and its gotchas.
- [`spoke-connect-libp2p-spike.md`](spoke-connect-libp2p-spike.md) - the rust-libp2p transport demonstration.
