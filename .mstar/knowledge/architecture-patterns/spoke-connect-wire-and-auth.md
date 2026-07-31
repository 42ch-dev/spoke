---
module: spoke-connect
date: 2026-07-31
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["adding a transport/interaction envelope family", "implementing a spoke-connect adapter or SDK", "designing network auth over libp2p for SPOKE"]
tags: [spoke-connect, connect-envelope, jcs-signature, noise-peerid, session-ordering, dual-identity, capability-flag]
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

**Dual identity**: `peer_id` (opaque network trust root - allowlist key) is distinct from `host_id` (manifest application label); they need not be equal. Session `initiator_peer_id`/`responder_peer_id` MUST equal the authenticated hello `peer_id`s; receivers reject mismatching session snapshots.

**Ordering** has two layers: message-level (per-session monotonic `sequence` from 0 per direction, no wrap - exhaustion closes session; `request_id` correlation/echo) and orchestration-level (manifest `roles`-driven peer selection and typical invocation order; explicitly not global consensus).

**Auth model**: `noise-peerid` - libp2p Noise gives transport encryption + peer identity (PeerId from keypair); the protocol layer adds a PeerId allowlist trust root and a signed hello. The auth envelope's open `method` string reserves extension points (`capability-token`, `did`) without implementing them; `proof` is opaque per method.

**Capability flag**: `spoke-connect` is an optional flag in `spoke-protocol-layers.md` (same opt-in pattern as `l2-computable` / `l5-fork` / `narrative-modules`); baseline MUST NOT require it. The baseline exclusions ("Transport bindings", "Network discovery") point at this opt-in family without dropping the exclusion semantics.

## Why This Matters

Without a standard interaction layer, every multi-adapter product reinvents handshake, auth, ordering, and remote-op envelopes - breaking cross-product interoperability that is SPOKE's north star. Locking the envelopes as wire SSOT with codegen (TS + Rust) means client adapters implement against a stable contract, and the signed-hello + allowlist model gives a defensible auth baseline that later methods (tokens, DID) extend without re-litigating the transport layer. Keeping it opt-in preserves the "protocol repo, not a runtime" boundary: the wire + normative spec live here; the embeddable SDK and product bindings ship separately.

## When to Apply

- Adding any new connect-family envelope or field - follow the closed-envelope + required-`extensions` + `$ref`-reuse rules; bump `protocol_version` for breaking shape changes (negotiated via hello).
- Implementing a spoke-connect adapter or SDK in any language - the JCS signed-field set, dual-identity rule, session peer-binding, and sequence semantics are normative; validate `const 0` / sequence range at the wire boundary (generated Rust types do not enforce `const`).
- Designing network auth over libp2p for SPOKE - start from `noise-peerid` + allowlist; gate at `ConnectionEstablished` (before disclosure); bind the verify key to the noise PeerId, not to self-reported hello fields.

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
