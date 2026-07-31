# SPOKE Connect

> **Status:** Normative — opt-in capability (`spoke-connect`)
> **Document class:** Detail — interaction **wire** family (opt-in)
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)
> **Schema home:** `schemas/connect/`, `schemas/common/`
> **Capability flag:** [`spoke-protocol-layers.md`](spoke-protocol-layers.md) §Optional flags

## Purpose

Define the **interaction envelopes** for cross-process SPOKE hosts: signed manifest exchange (hello), session context, remote op invocation, and extensible authentication. Connect is an **opt-in capability family** beside the three pillars (data wire, ops wire, ops library) — baseline compliance does not include it, and no baseline schema changes. Envelopes are transport-agnostic JSON payloads; transport binding is a stack concern (see §[Reference stack](#reference-stack)).

**Integrator framing:**

| Principle | Meaning |
|-----------|---------|
| **Reuse, don't redefine** | hello embeds `HostCapabilityManifest` by `$ref`; invoke failure reuses `error-envelope`; payload uses `OpaqueJson`; no new identity system beyond opaque `peer_id` |
| **Opaque payload wrapping** | invoke envelopes wrap existing ops envelopes as opaque JSON; connect does not re-specify upsert/check/assemble fields |
| **Closed envelopes, open vocabulary** | `additionalProperties: false`; `op`, `method`, and capability names stay open strings with documented core lists |
| **`extensions` required** | every connect envelope carries `ExtensionMap`; hello-level `extensions` are unsigned |
| **No engine fields** | no ranking, retrieval, routing scores, token budgets, multiaddrs, or DHT keys on connect wire |

## User value

| Without this spec | With this spec |
|-------------------|----------------|
| Multi-adapter client interaction has no standard wire | Integrators build connect adapters against a dual-language contract (`schemas/connect/` + generated types) |
| Ordering, auth, and discovery are invented per product | Per-session ordering, `noise-peerid` auth, and explicit peering are normative |
| Baseline-only hosts are forced to change | `spoke-connect` is an optional flag; baseline compliance is unchanged |

## Relationship to the existing three pillars

| Pillar | Path | Connect relationship |
|--------|------|----------------------|
| Data wire | `schemas/data/` | Unchanged. Hello **embeds** `HostCapabilityManifest` by `$ref` — does not fork or extend the data schema. |
| Ops wire | `schemas/ops/` | Unchanged. Invoke **wraps** ops request/response envelopes as `payload: OpaqueJson` — connect does not re-specify op fields. |
| Ops library | `@42ch/spoke-operations` / `spoke-operations` | Unchanged. Connect does not add pure helpers; remote dispatch stays adapter-owned. |
| **Connect family (opt-in)** | `schemas/connect/` + `spoke-connect` flag | Cross-process interaction envelopes + session ordering + auth model. Fourth concern beside the three pillars; baseline hosts ignore it. |

## Dual identity: `peer_id` vs `host_id`

| Identity | Wire location | Meaning | Trust role |
|----------|---------------|---------|------------|
| **`peer_id`** | connect envelopes | Opaque network peer identity string (libp2p PeerId multibase encoding in the reference stack; other stacks map equivalently) | **Trust root** for `noise-peerid` — allowlist matches `peer_id` |
| **`host_id`** | inside embedded `HostCapabilityManifest` | Opaque application host identity (existing data-model field) | **Advisory label** for orchestration / logging; NOT the allowlist key |

The protocol does **not** require `peer_id == host_id`. Deployments SHOULD document their mapping; receivers MUST authorize on `peer_id` (and signature), then consume the `host` manifest for roles/capabilities.

## Envelope family (`schemas/connect/`)

Six envelopes — all `type: object`, `additionalProperties: false`, required `extensions` → `$ref` `ExtensionMap`. `$id` base: `https://spoke42.invalid/schemas/` (same as the other families). Connect schemas evolve in place: breaking shape changes bump `protocol_version`, which peers negotiate via hello.

| File | `title` | `$id` suffix |
|------|---------|--------------|
| `connect-hello.schema.json` | `ConnectHello` | `connect/connect-hello.schema.json` |
| `connect-session.schema.json` | `ConnectSession` | `connect/connect-session.schema.json` |
| `connect-invoke-request.schema.json` | `ConnectInvokeRequest` | `connect/connect-invoke-request.schema.json` |
| `connect-invoke-response.schema.json` | `ConnectInvokeResponse` | `connect/connect-invoke-response.schema.json` |
| `connect-auth-challenge.schema.json` | `ConnectAuthChallenge` | `connect/connect-auth-challenge.schema.json` |
| `connect-auth-response.schema.json` | `ConnectAuthResponse` | `connect/connect-auth-response.schema.json` |

## Field tables

### ConnectHello — handshake / signed manifest exchange

| Field | Req | Type | Notes |
|-------|-----|------|-------|
| `protocol_version` | yes | integer ≥ 1 | Connect protocol version (not data `schema_version`). Protocol version **1** is current. |
| `peer_id` | yes | string, minLength 1 | Sender network identity (opaque). |
| `nonce` | yes | string, minLength 16 | Single-use replay nonce (see [§Nonce / replay](#nonce--replay-protection)). |
| `host` | yes | `$ref` HostCapabilityManifest | Full embedded manifest (includes `host.extensions`). |
| `signature` | yes | string, minLength 1 | base64url (no padding) of raw signature bytes (see [§Signature canonicalization](#signature-canonicalization-hello)). |
| `extensions` | yes | ExtensionMap | Product bag; **not** covered by signature. |

### ConnectSession — established session context (wire snapshot)

| Field | Req | Type | Notes |
|-------|-----|------|-------|
| `session_id` | yes | string, minLength 1 | Opaque session id (UUID recommended; not schema-enforced). |
| `initiator_peer_id` | yes | string, minLength 1 | Peer that dialed / sent first hello. |
| `responder_peer_id` | yes | string, minLength 1 | Peer that accepted. |
| `opened_at` | yes | Timestamp (RFC 3339) | Session open time (UTC). |
| `negotiated_capabilities` | yes | string[], minItems 1, uniqueItems | Intersection (or agreed subset) of both hosts' `capabilities[]`; MUST include `spoke-connect` when both declare it. |
| `initial_sequence` | yes | integer, minimum 0, **const 0** for protocol_version 1 | First invoke request uses `sequence = initial_sequence` (i.e. **0**). Runtime validators MUST enforce the `const 0` rule at the wire boundary; generated Rust types do not encode it. |
| `extensions` | yes | ExtensionMap | |

Session is a **wire-visible snapshot** (fixtures + optional session-announce messages). A runtime may keep richer local state; the public wire shape stays this table.

`initiator_peer_id` and `responder_peer_id` MUST equal the authenticated hello `peer_id` values of the two session peers; receivers MUST reject session snapshots whose peer ids do not match the authenticated hellos.

### ConnectInvokeRequest — remote op call

| Field | Req | Type | Notes |
|-------|-----|------|-------|
| `session_id` | yes | string, minLength 1 | |
| `sequence` | yes | integer, minimum 0, maximum 2⁵³−1 (JSON-safe); logical u64 | Monotonic per session **outbound** from this sender (see [§Ordering semantics](#ordering-semantics)). |
| `request_id` | yes | string, minLength 1 | Caller-generated correlation id (UUID recommended). |
| `op` | yes | string, minLength 1 | Open vocabulary — see [§`op` core vocabulary](#op-core-vocabulary). |
| `payload` | yes | `$ref` OpaqueJson | Opaque JSON — MUST be a full existing ops **request** envelope for the named `op` when targeting SPOKE ops. Dispatchers MUST validate the payload against the named `op`'s ops request schema. |
| `auth` | no | `$ref` OpaqueJson | Optional mid-session proof blob; primary auth is hello. Shape method-specific when used. |
| `extensions` | yes | ExtensionMap | |

### ConnectInvokeResponse — remote op reply

Mirrors ops response style: **oneOf** success | error (no parallel `status` enum — discriminator is `payload` vs `error`).

**Success branch** — required: `session_id`, `sequence`, `request_id`, `payload`, `extensions`

| Field | Req | Type | Notes |
|-------|-----|------|-------|
| `session_id` | yes | string | Echo. |
| `sequence` | yes | integer ≥ 0 | Echo of request `sequence`. |
| `request_id` | yes | string | Echo of request `request_id`. |
| `payload` | yes | OpaqueJson | Ops **response** success envelope (or product-defined success body for non-core `op`). |
| `extensions` | yes | ExtensionMap | |

**Error branch** — required: `session_id`, `sequence`, `request_id`, `error`, `extensions`

| Field | Req | Type | Notes |
|-------|-----|------|-------|
| `session_id` / `sequence` / `request_id` | yes | (echo) | Same as success. |
| `error` | yes | `$ref` ErrorEnvelope | Shared failure shape — **no** parallel connect error object. |
| `extensions` | yes | ExtensionMap | |

Branches MUST NOT include both `payload` and `error`.

### ConnectAuthChallenge

| Field | Req | Type | Notes |
|-------|-----|------|-------|
| `challenge_id` | yes | string, minLength 1 | Correlation id for the response. |
| `method` | yes | string, minLength 1 | Open vocabulary — core: `noise-peerid` (see [§`method` core vocabulary](#method-core-vocabulary)). |
| `challenge` | yes | string, minLength 1 | Opaque challenge material (nonce / method-specific). |
| `extensions` | yes | ExtensionMap | |

### ConnectAuthResponse

| Field | Req | Type | Notes |
|-------|-----|------|-------|
| `challenge_id` | yes | string | Echo. |
| `method` | yes | string | MUST match challenge `method`. |
| `proof` | yes | OpaqueJson | Method-specific proof (signature bundle, token, …). Unused for `noise-peerid` — hello is the proof; other methods validate `proof` per method. |
| `extensions` | yes | ExtensionMap | |

## Signature canonicalization (hello)

**Algorithm name:** `spoke-connect-hello-jcs-v1`

1. Build the **signed object** (JSON object, exactly these keys — no others):

```json
{
  "protocol_version": <integer>,
  "peer_id": <string>,
  "nonce": <string>,
  "host": <HostCapabilityManifest object including host.extensions>
}
```

2. Canonicalize with **RFC 8785 JSON Canonicalization Scheme (JCS)** → UTF-8 bytes.
3. Sign those bytes with the **peer identity private key** that corresponds to `peer_id` (reference stack: libp2p identity keypair; Ed25519 in current rust-libp2p default).
4. Encode the raw signature bytes as **base64url without padding** → `signature` field.

**Included in signature:** `protocol_version`, `peer_id`, `nonce`, entire `host` (including `host.extensions`).

**Excluded from signature:** top-level hello `extensions`, and the `signature` field itself.

Receivers MUST NOT use top-level hello `extensions` for authorization or trust decisions; only the signed fields and the embedded `host` manifest participate in `noise-peerid` accept/reject.

**Verify:** recompute JCS over the same four fields from the received hello; verify the signature with the public key derived from / bound to `peer_id`; reject on mismatch.

**Why JCS:** multi-language SDKs (Rust / future uniffi targets) need one deterministic byte sequence; JCS is standardized (RFC 8785) and library-backed. Implementations MUST NOT invent field-order rules.

## Nonce / replay protection

| Rule | Value |
|------|-------|
| `nonce` entropy | minLength 16 is a wire floor; generators SHOULD use ≥128 bits CSPRNG (encoded output typically exceeds 16 chars) |
| Scope | Per `peer_id` of the **sender** |
| Uniqueness | Receiver MUST reject a hello whose `(peer_id, nonce)` pair was already accepted |
| Replay window | In-memory set for the life of the process is sufficient for the reference spike; products MAY persist nonces with TTL — not specified in this protocol version |
| Bound into signature | Yes — nonce is inside the signed object |

## Ordering semantics

### Message layer

| Rule | Value |
|------|-------|
| Sequence domain | Non-negative integer; protocol_version 1 starts at **`initial_sequence = 0`** |
| First invoke | `sequence = 0` |
| Increment | Each new **outbound** `ConnectInvokeRequest` from a peer in a session uses `last_sequence + 1` |
| Direction | Each peer maintains its **own** outbound counter; sequences are not a single shared total-order across both directions |
| Correlation | Response MUST echo `sequence` and `request_id` from the request |
| Timeout / retry | Timeout, retry, and duplicate detection are adapter-owned; `request_id` is the correlation handle for retries — protocol v1 defines no retry semantics |
| Concurrent invokes | Allowed. Assign sequence atomically at send time. Completions MAY arrive out of order; apps that need FIFO processing sort/buffer by `sequence` |
| Overflow | When next sequence would exceed **2⁵³−1** (the JSON-safe wire maximum in the [§ConnectInvokeRequest](#connectinvokerequest--remote-op-call) field table) or the implementation counter limit, peer MUST close the session and open a new one — **no wrap-around** |
| Guarantee scope | Per-session message-layer only — **not** global consensus, not cross-session ordering |

### Orchestration layer

Normative guidance, not a wire field: hosts select peers using remote `HostCapabilityManifest.roles` / `capabilities` / `namespaces` (e.g. prefer `roles` containing `checker` for `op: "check"`). Typical order: connect → exchange hello → establish session → invoke. Explicitly **not** distributed transaction or total-order broadcast.

## `op` core vocabulary

Open string (documented, not JSON Schema enum):

| Core `op` | Maps to ops family | Notes |
|-----------|-------------------|-------|
| `upsert` | upsert-request / upsert-response | Baseline |
| `promote` | promote-* | Baseline |
| `relate` | relate-* | Baseline |
| `check` | check-* | Baseline |
| `assemble` | assemble-* | Baseline |
| `project` | project-* | Optional; meaningful when remote declares `l2-computable` |
| `compute` | compute-* | Optional; same |

Unknown `op` values are valid on the wire; receivers return `ErrorEnvelope` with an appropriate `code` (e.g. `op_unsupported`) when they cannot handle them. Connect does not close the vocabulary. This vocabulary MUST stay in sync with the schema `description` fields.

## `method` core vocabulary

Open string (documented, not enum):

| `method` | Status |
|----------|--------|
| `noise-peerid` | **Normative core** — PeerId allowlist + signed hello |
| `capability-token` | Reserved name only — not implemented |
| `did` | Reserved name only — not implemented |

This vocabulary MUST stay in sync with the schema `description` fields.

## Auth model

**Core method — `noise-peerid`:** the trust root is a deployment-configured `PeerId` allowlist. Authentication happens at hello: the sender signs the canonicalized signed object with the key bound to `peer_id`; the receiver accepts only when (a) `peer_id` is on the allowlist and (b) the signature verifies. `ConnectAuthChallenge` / `ConnectAuthResponse` exist so future methods (reserved names: `capability-token`, `did`) and step-up auth share one shape; `method` is an open string and `proof` is opaque. Accept/reject decisions use only the signed fields and the embedded `host` manifest; top-level hello `extensions` MUST NOT influence authorization.

A handshake MAY complete with hello alone — no extra auth round-trip — when allowlist + signature verification succeed. The challenge/response pair is not required for `noise-peerid` sessions. In protocol v1, handshake rejection closes the connection or stream; there is no hello error envelope.

## Discovery boundary

- Connect **wire** has no mDNS/DHT/multiaddr fields.
- Normative discovery path: **explicit peering** (configured addresses / out-of-band dial).
- mDNS is a **runtime convenience** (non-default feature of the reference stack) for same-LAN development only — it is not implied as production discovery.

## Reference stack

The reference stack maps these envelopes onto rust-libp2p: **noise** for authenticated transport, **yamux** for stream multiplexing, **request-response** for invoke, **identify** for peer metadata. `peer_id` uses the libp2p PeerId multibase encoding; other stacks map peer identity, sessions, and ordering equivalently. The reference spike lives in `crates/spoke-connect` (unpublished, `publish = false`) and demonstrates the mapping; it is a transport demonstration, not a protocol surface. Kademlia / DHT discovery is not part of this protocol version.

## Hard boundaries

| Excluded concern | Where it lives |
|------------------|----------------|
| Daemon / shared runtime | Protocol repo is wire + spec only; runtimes are product-owned (reference spike `crates/spoke-connect` stays unpublished) |
| Ranking / retrieval / routing scores | Product-local |
| DHT / NAT traversal (Kademlia, Gossipsub, circuit relay) | Not in this protocol version; wire carries no multiaddr/DHT fields |
| Op semantics (upsert/check/assemble fields) | Ops wire (`schemas/ops/`) — invoke `payload` is opaque |
| Session / nonce / dispatch persistence | Adapter-owned — wire defines envelope shapes and ordering rules, not storage |
| Invoke timeout / retry / duplicate detection | Adapter-owned; `request_id` is the correlation handle for retries; protocol v1 defines no retry semantics |
| Payload size / flow control | Wire imposes no payload size limit; size bounding and flow control are transport-adapter-owned |

**Design rules (hard):**

1. **Reuse, don't redefine** — hello embeds `HostCapabilityManifest` by `$ref`; invoke failure reuses `error-envelope`; payload uses `OpaqueJson`; no new identity system beyond opaque `peer_id`.
2. **Opaque payload wrapping** — invoke envelopes wrap existing ops envelopes as opaque JSON; connect does NOT re-specify upsert/check/assemble fields.
3. **Closed envelopes, open vocabulary** — `additionalProperties: false`; `op`, `method`, capability names stay open strings with documented core lists.
4. **`extensions` (ExtensionMap) required** on every connect envelope; hello-level `extensions` are unsigned.
5. **No engine fields** — no ranking, retrieval, routing scores, token budgets, multiaddrs, or DHT keys on connect wire.

## Acceptance (connect spec)

- [ ] Six connect envelopes committed under `schemas/connect/` — closed, required `extensions`
- [ ] Hello embeds `HostCapabilityManifest` via `$ref`; invoke-response error path reuses `error-envelope`
- [ ] Field tables in this spec match the schema files
- [ ] Signature algorithm named (`spoke-connect-hello-jcs-v1`, RFC 8785 JCS) with the exact signed field set
- [ ] Ordering rules complete: start at 0, per-direction counters, echo correlation, no wrap-around
- [ ] `spoke-connect` optional flag registered in `spoke-protocol-layers.md`; baseline unchanged
- [ ] Discovery default is explicit peering; mDNS described as non-default convenience only
- [ ] No transport-specific fields (multiaddr, DHT keys, HTTP/gRPC mapping) in connect schemas

## Non-goals (connect)

- Runtime / networking code (reference-stack spike is separate and unpublished)
- `capability-token` / DID auth methods (envelope reserves `method`; only `noise-peerid` is normative)
- Kademlia / DHT discovery, Gossipsub, circuit relay / NAT traversal on the wire
- mDNS as a wire concept or default discovery mechanism
- TypeScript connectivity SDK or binding decisions
- Publishing anything (no npm / crates.io changes for connect)
- Changing baseline capability requirements or existing ops schemas
- Re-defining `HostCapabilityManifest` fields (embed by `$ref` only)

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella framing and schema inventory |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8, capability levels, Optional flags (`spoke-connect`) |
| [`spoke-data-model.md`](spoke-data-model.md) | `HostCapabilityManifest` field tables (embedded by hello) |
| [`spoke-ops.md`](spoke-ops.md) | Ops wire + error envelope (wrapped as opaque invoke payloads) |
| [`spoke-operations.md`](spoke-operations.md) | Lifecycle helpers; adapter ports; `HostManifestPort` |
| [`schemas/README.md`](../../schemas/README.md) | Connect schema files under `schemas/connect/` |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Connect envelope family / Session (connect) / peer_id vocabulary |
| [`STRATEGY.md`](../../STRATEGY.md) | Protocol-not-runtime positioning |
