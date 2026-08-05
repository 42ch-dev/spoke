# SPOKE Connect

> **Status:** Normative — opt-in capability (`spoke-connect`)
> **Document class:** Detail — interaction **wire** family (opt-in)
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)
> **Schema home:** `schemas/connect/`, `schemas/common/`
> **Capability flag:** [`spoke-protocol-layers.md`](spoke-protocol-layers.md) §Optional flags

## Purpose

Define the **interaction envelopes** for cross-process SPOKE hosts: signed manifest exchange (hello), session context, remote op invocation, and extensible authentication. Connect is an **opt-in capability family** beside the three pillars (data wire, ops wire, ops library) — baseline compliance does not include it, and no baseline schema changes. Envelopes are transport-agnostic JSON payloads; transport binding is a stack concern (see §[Embedding model](#embedding-model), §[Transport framing](#transport-framing), §[Reference stack](#reference-stack)).

**Integrator framing:**

| Principle | Meaning |
|-----------|---------|
| **Reuse, don't redefine** | hello embeds `HostCapabilityManifest` by `$ref`; invoke failure reuses `error-envelope`; payload uses `OpaqueJson`; no new identity system beyond opaque `peer_id` |
| **Opaque payload wrapping** | invoke envelopes wrap existing ops envelopes as opaque JSON; connect does not re-specify upsert/check/assemble fields |
| **Closed envelopes, open vocabulary** | `additionalProperties: false`; `op`, `method`, and capability names stay open strings with documented core lists |
| **`extensions` required** | every connect envelope carries `ExtensionMap`; hello-level `extensions` are unsigned |
| **No engine fields** | no ranking, retrieval, routing scores, token budgets, multiaddrs, or DHT keys on connect wire |

## Embedding model

Connect is an **embeddable library contract**: wire shapes plus pure session rules that any language stack can implement. Product runtimes own transport and process lifecycle.

**Three layers:**

| Layer | What it is | Ownership |
|-------|------------|-----------|
| **Wire contract** | `schemas/connect/` envelopes + this specification (field tables, JCS, identity binding, framing unit, vocabularies) | Protocol SSOT — language-independent |
| **Session core** | Pure logic: hello accept/reject gates, nonce single-use, session state, per-direction sequence allocation and check, `request_id` correlation, allowlist evaluation, op dispatch gate | Language-portable rules; implementable without a shared native library |
| **Transport adapter** | Byte-stream or RPC channel, authenticated transport identity when present, stream lifecycle, dialing, peer metadata (e.g. identify), delimiter choice | Per language and per product |

**Two embedding paths** (internal labels **Path A** / **Path B**). Consumer-facing docs (`docs/`, root READMEs) use the aliases in the table — not the Path letters.

| Path (internal) | Consumer docs alias | Description | Current shipped surfaces |
|-----------------|---------------------|-------------|--------------------------|
| **Path A — language-direct** | **Language-native client** | Implement the wire contract and session-core rules in the host language; pair with that language’s network stack (js-libp2p, native sockets, WebSocket, etc.). No Rust runtime required. | TypeScript `@42ch/spoke-connect` (WebSocket) |
| **Path B — shared core bindings** | **Native bindings** | Export a session-core implementation (Rust via uniffi) into the host language; the transport adapter may stay in the host language or in the shared core. | C# NuGet `42ch.Spoke.Connect`, Kotlin Maven `dev.42ch:spoke-connect`, Swift SPM `SpokeConnect`, Go modules under `crates/spoke-connect/bindings/go`, Python PyPI `spoke-connect` |

**Rust reference (`spoke-connect` on crates.io):** the published crate is the **session-core reference** and the **Path B binding source** (uniffi facade + `bindings/**`). It also ships a rust-libp2p transport stack. It is **not** Path A — Path A means a host-language port with no Rust runtime. Consumer docs call this the **Rust reference**, not Path A or Path B.

Both paths MUST produce the same signed hello bytes, the same `peer_id` derivation, and the same session-core accept/reject outcomes for a given input. The rust-libp2p stack under `crates/spoke-connect` is one transport demonstration; it is evidence, not the definition of the rules.

### RemoteAdapter layer (above session-core)

A **RemoteAdapter** implements the async `BaselinePorts` adapter contract by proxying each port call as a reserved `port.*` op invoke over an established connect session. It is an opt-in surface in both connect packages — the `./remote` subpath in `@42ch/spoke-connect`, the `remote-adapter` cargo feature in `spoke-connect` — and reuses session-core (hello, allowlist, nonce, sequence, correlation, dispatch awareness, capability-token) without widening the TS↔Rust session-core parity table. All connect verification is encapsulated; consumers pass the adapter straight to the baseline orchestrators. Message-oriented `Transport` seam, port-method catalogue, error mapping, and concurrency rules: [spoke-remote-adapter.md](spoke-remote-adapter.md).

## User value

| Without this spec | With this spec |
|-------------------|----------------|
| Multi-adapter client interaction has no standard wire | Integrators build connect adapters against a dual-language contract (`schemas/connect/` + generated types) |
| Ordering, auth, and discovery are invented per product | Per-session ordering, `noise-peerid` + `capability-token` auth, and explicit peering are normative |
| Non-Rust clients reverse-engineer a single stack | Identity, framing, and session-core rules are stack-agnostic and implementable from this spec alone |
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
| **`peer_id`** | connect envelopes | Network peer identity string derived per §[Identity binding](#identity-binding) (protocol_version 1: Ed25519 libp2p identity PeerId) | **Trust root** for `noise-peerid` — allowlist matches `peer_id` |
| **`host_id`** | inside embedded `HostCapabilityManifest` | Opaque application host identity (existing data-model field) | **Advisory label** for orchestration / logging; NOT the allowlist key |

The protocol does **not** require `peer_id == host_id`. Deployments SHOULD document their mapping; receivers MUST authorize on `peer_id` (and signature), then consume the `host` manifest for roles/capabilities.

## Identity binding

Normative mapping for protocol_version **1**. Implementers in any language derive and verify identity from this section alone.

### Key type

| Item | Value |
|------|-------|
| Public-key algorithm | **Ed25519** |
| Raw public key | 32 bytes |
| Raw signature | 64 bytes |
| Private key | Ed25519 keypair used to sign hello (see §[Signature canonicalization](#signature-canonicalization-hello)) |

### Signature algorithm

**Name:** `spoke-connect-hello-jcs-v1` (unchanged).

Role-aware signed-field set (the role is carried by the hello itself — `peer_nonce` present ⇒ responder):

| Role | Signed object (exact keys) |
|------|----------------------------|
| **Initiator** hello | `{protocol_version, peer_id, nonce, host}` (4 fields — `peer_nonce` absent; the initiator sends first and has no peer nonce) |
| **Responder** hello | `{protocol_version, peer_id, nonce, host, peer_nonce}` (5 fields — `peer_nonce` = the initiator's nonce, **dial binding**) |

1. Build the signed object per the table above (see §[Signature canonicalization](#signature-canonicalization-hello)).
2. Canonicalize with **RFC 8785 JCS** → UTF-8 bytes.
3. `Ed25519.sign(private_key, jcs_bytes)` → 64 raw bytes.
4. Encode as **base64url without padding** → wire `signature`.

Verify recomputes JCS over the same role-aware field set from the received hello and verifies with the public key that derives `peer_id`. An initiator that verifies a **responder** hello MUST additionally assert `peer_nonce == <its own nonce>` — a responder hello signed over a different or absent initiator nonce is rejected (**dial binding**; defeats replay of a captured responder hello across a client restart, see [§Nonce / replay](#nonce--replay-protection)).

### `peer_id` derivation (libp2p identity / peer-ids)

Wire `peer_id` is the **libp2p PeerId string** for the Ed25519 public key, per the published libp2p identity / peer-ids specification. Steps:

1. Encode the public key as the libp2p protobuf **`PublicKey`** message:
   - field 1 (`Type`) = `1` (**Ed25519**);
   - field 2 (`Data`) = the **32-byte** raw Ed25519 public key.
2. Let `pk_bytes` = the protobuf serialization of that message. For Ed25519, `pk_bytes` length is **≤ 42 bytes**.
3. Build the **multihash wire byte sequence** over `pk_bytes` using the **identity** multihash code **`0x00`**:

   ```text
   multihash_bytes = code-varint (0x00 identity) || length-varint || digest
   ```

   where `digest` is `pk_bytes` itself (the protobuf-encoded `PublicKey` bytes — **not** a cryptographic hash of them, and **not** the raw 32-byte public key alone).  
   **Do not** hash the raw 32-byte public key with sha2-256 for this step.  
   *(General libp2p rule: identity multihash when the protobuf public key is ≤ 42 bytes; sha2-256 multihash only when longer — e.g. RSA. Protocol_version 1 connect cores that support only Ed25519 always take the identity branch.)*
4. Encode `multihash_bytes` with **base58btc** (Bitcoin alphabet). That string is wire `peer_id`.

**Encoding constraints:**

- **No** multibase prefix character (no leading `z`).
- **No** CIDv1 wrapping.
- The result matches rust-libp2p `PeerId::to_string()` / `to_base58()` for the same key.

### Binding rules

| Rule | Requirement |
|------|-------------|
| Authorization | Receivers authorize on `peer_id` + verified signature; `host_id` remains advisory |
| Authenticated transport | When the transport supplies an authenticated peer identity (reference: Noise remote peer id), the claimed hello `peer_id` MUST equal that transport-authenticated id, and the verify public key MUST derive that same `peer_id` |
| Path A without Noise | Stacks still MUST derive `peer_id` per steps 1–4 and MUST verify the hello signature against the key that derives that `peer_id`. How the public key is obtained (hello-adjacent identify, preconfiguration, etc.) is transport-adapter-owned |
| Signed-field set | Role-aware — initiator hello: `protocol_version`, `peer_id`, `nonce`, `host` (4 fields); responder hello: the same plus `peer_nonce` = the initiator's nonce (5 fields, dial binding) |

This section does not define JWT/DID key ids and does not require `peer_id == host_id`.

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
| `peer_id` | yes | string, minLength 1 | Sender network identity (see §[Identity binding](#identity-binding)). |
| `nonce` | yes | string, minLength 16 | Single-use replay nonce (see [§Nonce / replay](#nonce--replay-protection)). |
| `peer_nonce` | no | string, minLength 16 | Responder-only dial binding: the initiator's nonce, echoed by the responder and bound into its signed object. Absent in initiator hellos; initiators MUST reject a responder hello whose `peer_nonce` does not equal their own nonce (see [§Nonce / replay](#nonce--replay-protection)). |
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
| `auth` | no | `$ref` OpaqueJson | Optional mid-session proof blob; primary session identity is hello (`noise-peerid`). For `capability-token`, same proof object as auth response (see [§Method — capability-token](#method--capability-token)). |
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
| `method` | yes | string, minLength 1 | Open vocabulary — core: `noise-peerid`, `capability-token` (see [§`method` core vocabulary](#method-core-vocabulary)). |
| `challenge` | yes | string, minLength 1 | Opaque challenge material (nonce / method-specific). |
| `extensions` | yes | ExtensionMap | |

### ConnectAuthResponse

| Field | Req | Type | Notes |
|-------|-----|------|-------|
| `challenge_id` | yes | string | Echo. |
| `method` | yes | string | MUST match challenge `method`. |
| `proof` | yes | OpaqueJson | Method-specific proof. Unused for `noise-peerid` (hello is the proof). For `capability-token`: `{ v, claims, sig }` object (see [§Method — capability-token](#method--capability-token)). |
| `extensions` | yes | ExtensionMap | |

## Signature canonicalization (hello)

**Algorithm name:** `spoke-connect-hello-jcs-v1`

**Key type (protocol_version 1):** Ed25519 — see §[Identity binding](#identity-binding).

1. Build the **signed object** (JSON object, exactly these keys — no others). The field set is **role-aware**: the initiator signs 4 fields, the responder signs 5 fields (dial binding):

**Initiator hello** (signed object):

```json
{
  "protocol_version": <integer>,
  "peer_id": <string>,
  "nonce": <string>,
  "host": <HostCapabilityManifest object including host.extensions>
}
```

**Responder hello** (signed object — `peer_nonce` = the initiator's nonce):

```json
{
  "protocol_version": <integer>,
  "peer_id": <string>,
  "nonce": <string>,
  "host": <HostCapabilityManifest object including host.extensions>,
  "peer_nonce": <string>
}
```

2. Canonicalize with **RFC 8785 JSON Canonicalization Scheme (JCS)** → UTF-8 bytes.
3. Sign those bytes with the **Ed25519 peer identity private key** whose public key derives `peer_id` per §[Identity binding](#identity-binding).
4. Encode the raw 64-byte signature as **base64url without padding** → `signature` field.

**Included in signature:** `protocol_version`, `peer_id`, `nonce`, entire `host` (including `host.extensions`); for the responder role additionally `peer_nonce`.

**Excluded from signature:** top-level hello `extensions`, and the `signature` field itself.

Receivers MUST NOT use top-level hello `extensions` for authorization or trust decisions; only the signed fields and the embedded `host` manifest participate in `noise-peerid` accept/reject.

**Verify (role-aware):** recompute JCS over the signed-field set indicated by the received hello — 4 fields when `peer_nonce` is absent (initiator hello), 5 fields when it is present (responder hello); verify the signature with the public key that derives `peer_id` (identity multihash of protobuf `PublicKey`, base58btc); reject on mismatch. An initiator verifying a responder hello MUST additionally assert `peer_nonce` equals its own nonce (dial binding). Fail-closed: an initiator dial that expects a responder MUST reject a hello WITHOUT `peer_nonce` — a mixed-version old responder omits the field, and accepting it would skip the dial-binding assert.

**Why JCS:** multi-language SDKs need one deterministic byte sequence; JCS is standardized (RFC 8785) and library-backed. Implementations MUST NOT invent field-order rules.

## Nonce / replay protection

| Rule | Value |
|------|-------|
| `nonce` entropy | minLength 16 is a wire floor; generators SHOULD use ≥128 bits CSPRNG (encoded output typically exceeds 16 chars) |
| Scope | Per `peer_id` of the **sender** |
| Uniqueness | Receiver MUST reject a hello whose `(peer_id, nonce)` pair was already accepted |
| Replay window | In-memory set for the life of the process is sufficient for the reference stack; products MAY persist nonces with TTL — not specified in this protocol version |
| Bound into signature | Yes — nonce is inside the signed object |
| Rejected hello | Nonce of a **rejected** hello MUST NOT be recorded (retry-safe) |
| **Dial binding** | A responder hello MUST carry `peer_nonce` = the initiator's nonce (5-field signed object). The initiator MUST reject a responder hello whose signed `peer_nonce` differs from its own nonce, and MUST reject a responder hello WITHOUT `peer_nonce` (fail-closed — an old/mixed-version responder that omits the field must not bypass the binding). This defeats replay of a captured responder hello **across a client restart** — the scenario the in-memory `(peer_id, nonce)` store alone cannot cover, because the store resets with the process |

## Ordering semantics

### Message layer

| Rule | Value |
|------|-------|
| Sequence domain | Non-negative integer; protocol_version 1 starts at **`initial_sequence = 0`** |
| First invoke | `sequence = 0` |
| Increment | Each new **outbound** `ConnectInvokeRequest` from a peer in a session uses `last_sequence + 1` |
| Direction | Each peer maintains its **own** outbound counter; sequences are not a single shared total-order across both directions |
| Correlation | Response MUST echo `session_id`, `sequence`, and `request_id` from the request |
| Timeout / retry | Timeout, retry, and duplicate detection are adapter-owned; `request_id` is the correlation handle for retries — protocol v1 defines no retry semantics |
| Concurrent invokes | Allowed. Assign sequence atomically at send time. Completions MAY arrive out of order; apps that need FIFO processing sort/buffer by `sequence` |
| Overflow | When next sequence would exceed **2⁵³−1** (the JSON-safe wire maximum in the [§ConnectInvokeRequest](#connectinvokerequest--remote-op-call) field table) or the implementation counter limit, peer MUST close the session and open a new one — **no wrap-around** |
| Guarantee scope | Per-session message-layer only — **not** global consensus, not cross-session ordering |

### Orchestration layer

Hosts select peers using remote `HostCapabilityManifest.roles` / `capabilities` / `namespaces` (e.g. prefer `roles` containing `checker` for `op: "check"`). That peer-selection policy is **guidance** for product orchestration.

Typical order: connect → exchange hello → establish session → invoke.

**Op dispatch** is **MUST**-level and is defined in §[Op dispatch gate](#op-dispatch-gate): before running a handler, the required capability for `op` MUST be present in the session’s `negotiated_capabilities`.

Orchestration over connect is **not** distributed transaction and **not** total-order broadcast across sessions.

## Session-core state machine

Logical states and transitions for the **session core** (pure rules). Dialing, Noise, identify buffering, and stream lifecycle are **transport-adapter-owned**.

### States (per local node, per session)

| State | Meaning |
|-------|---------|
| `Disconnected` | No transport session; no outbound sequence for this session; nonce store may still be process-global |
| `Handshaking` | Transport up; hellos in flight; invokes are not yet authorized |
| `Established` | Both hellos accepted (or local policy equivalent); `session_id` assigned locally; outbound counter = 0; inbound expected = 0 |
| `Closed` | Session unusable (sequence exhaustion, transport loss, auth failure, local shutdown); open a new session — no sequence wrap |

### Transitions (MUST)

| From → To | Trigger | Guards / effects |
|-----------|---------|------------------|
| `Disconnected` → `Handshaking` | Transport connect/accept | — |
| `Handshaking` → `Established` | Local accept of remote hello **and** remote accept of local hello (both directions confirmed) | Allowlist + signature + nonce single-use; dial binding — the responder's signed `peer_nonce` equals the initiator's own nonce; record `(peer_id, nonce)` only on accept; bind session peer ids to authenticated hello `peer_id`s; `negotiated_capabilities` = agreed subset including `spoke-connect` when both declare it; set outbound counter = 0 and inbound expected = 0 |
| `Handshaking` → `Closed` (or disconnect) | Any hello gate failure | Nonce of **rejected** hello MUST NOT be recorded; no trust from unsigned hello `extensions` |
| `Established` → `Established` | Outbound invoke | Atomically allocate `sequence = last + 1` starting at 0; attach a new `request_id`; send |
| `Established` → `Established` | Inbound invoke | Accept iff `sequence == next_expected_inbound` (start 0); then `next_expected_inbound += 1`; else reject with wire error and **no** handler side effect |
| `Established` → `Established` | Inbound response | Accept iff it echoes `session_id`, `sequence`, and `request_id` of a pending request; else correlation failure (local error) |
| `Established` → `Closed` | Next outbound sequence would exceed 2⁵³−1, or transport loss, or local shutdown | No wrap-around |

These labels and guards are the portable session-core contract so Path A ports match reference behavior without depending on reference-crate internals.

## `op` core vocabulary

Open string (documented, not JSON Schema enum). Schema `description` on `ConnectInvokeRequest.op`: core values `upsert`, `promote`, `relate`, `check`, `assemble`, `project`, `compute`.

| Core `op` | Maps to ops family | Notes |
|-----------|-------------------|-------|
| `upsert` | upsert-request / upsert-response | Baseline |
| `promote` | promote-* | Baseline |
| `relate` | relate-* | Baseline |
| `check` | check-* | Baseline |
| `assemble` | assemble-* | Baseline |
| `project` | project-* | Optional; meaningful when remote declares `l2-computable` |
| `compute` | compute-* | Optional; same |

Unknown `op` values are valid on the wire; receivers return `ErrorEnvelope` with an appropriate `code` (e.g. `op_unsupported`) when they cannot handle them. Connect does not close the vocabulary. This vocabulary MUST stay in sync with the schema `description` fields under `schemas/connect/` and the corresponding ops schema text under `schemas/ops/`.

## Op dispatch gate

Before executing or forwarding a `ConnectInvokeRequest`, a host that performs op dispatch MUST ensure the capability required by `op` is present in the session’s `negotiated_capabilities`. If the required capability is absent, the host MUST NOT run the op handler and MUST answer with a `ConnectInvokeResponse` error branch (`ErrorEnvelope`, e.g. code `op_unsupported` or `capability_missing`) — no side effects.

For protocol_version 1 core ops, required capabilities are:

| `op` | Required capability (minimum) |
|------|-------------------------------|
| `upsert`, `promote`, `relate`, `check`, `assemble` | `spoke-baseline` (or a deployment-documented synonym that both peers list and intersect into `negotiated_capabilities`) |
| `project`, `compute` | `l2-computable` |

Product-defined `op` values MUST document their required capability name(s). The gate is evaluated against **`negotiated_capabilities`**, not against the remote manifest alone and not against unsigned hello `extensions`.

Sequence and correlation checks (§[Session-core state machine](#session-core-state-machine)) apply before or with this gate; a failed sequence check also produces no handler side effect.

## Ops-family registration

A new or modified ops family that is intended to be remotely invokable over connect MUST:

1. Define stable `op` string(s) and keep them listed in this spec’s §[`op` core vocabulary](#op-core-vocabulary) **or** in product documentation if outside core;
2. Keep those strings in sync with the corresponding JSON Schema `description` fields under `schemas/ops/` (and connect schema text that cites them);
3. Declare the capability name(s) hosts must advertise in `HostCapabilityManifest.capabilities` for the family to be negotiable;
4. Rely on opaque `payload` wrapping of existing ops request/response envelopes — **MUST NOT** require connect envelope shape changes for new ops fields.

Connect schemas and the six envelope shapes stay closed; extensibility is vocabulary + capabilities + opaque payload, not new connect fields.

## `method` core vocabulary

Open string (documented, not enum). Schema `description` on `ConnectAuthChallenge.method`: core `noise-peerid`, `capability-token`; reserved name `did`.

| `method` | Status |
|----------|--------|
| `noise-peerid` | **Normative** — PeerId allowlist + signed hello |
| `capability-token` | **Normative** — signed capability token proof + validation rules (see [§Method — capability-token](#method--capability-token)) |
| `did` | Reserved name only — not implemented |

This vocabulary MUST stay in sync with the schema `description` fields.

## Auth model

Connect authentication is **method-extensible**: `ConnectAuthChallenge` / `ConnectAuthResponse` share one envelope shape (`method` open string, `proof` opaque). Protocol_version 1 defines two normative methods — `noise-peerid` (session identity at hello) and `capability-token` (step-up / delegated capability grant). Reserved name `did` stays unimplemented. Accept/reject for hello uses only the signed fields and the embedded `host` manifest; top-level hello `extensions` MUST NOT influence authorization.

### Method — noise-peerid

The trust root is a deployment-configured `PeerId` allowlist. Authentication happens at hello: the sender signs the canonicalized signed object with the Ed25519 key bound to `peer_id` (see §[Identity binding](#identity-binding)); the receiver accepts only when (a) `peer_id` is on the allowlist and (b) the signature verifies against the public key that derives that `peer_id`.

A handshake MAY complete with hello alone — no extra auth round-trip — when allowlist + signature verification succeed. The challenge/response pair is not required for `noise-peerid` sessions. In protocol v1, handshake rejection closes the connection or stream; there is no hello error envelope. `capability-token` is **step-up / mid-session** authorization on top of an established `noise-peerid` session; it does not replace the hello identity gate.

### Method — capability-token

`capability-token` authenticates a **short-lived, capability-scoped grant** issued by a trusted issuer. The wire carries the token as the opaque `proof` on `ConnectAuthResponse` and optionally on `ConnectInvokeRequest.auth`. Token validation is **offline**: signature + trusted-issuer list + subject/audience/expiry + capability membership. There is no revocation list, no refresh token, and no `/token` issuance endpoint in this protocol version.

#### Token claim set

The **signed claims object** is canonicalized with **RFC 8785 JCS** → UTF-8 bytes, then signed with the **issuer** Ed25519 private key (same key class as hello: raw 64-byte Ed25519 signature → base64url without padding). Signature covers **only** JCS(`claims`) — not the wire wrapper.

| Field | Req | Type | Semantics |
|-------|-----|------|-----------|
| `iss` | yes | string | Issuer `peer_id` (same string form as hello `peer_id`) |
| `sub` | yes | string | Subject `peer_id` — who may present the token |
| `aud` | yes | string | Audience — receiver `peer_id` (reference implementation); products MAY document a `host_id` synonym later |
| `capabilities` | yes | string[] | Capability names granted to `sub` (e.g. `spoke-baseline`, `l2-computable`) |
| `exp` | yes | number (JSON integer) | Expiry as **Unix time seconds** (UTC); reject if `now >= exp` |
| `iat` | no | number | Issued-at Unix seconds; if present, reject if `now < iat` (reference implementations SHOULD allow ±60s clock skew on both sides) |
| `jti` | no | string | Unique id; if present, MUST be non-empty. Not consulted by protocol_version 1 validation — reserved for a future revocation design |

**No other claim keys** in protocol_version 1. Unknown claim keys in the signed object: **reject** (fail closed) so JCS bytes stay intentional.

#### Wire `proof` wrapper

`proof` is an **OpaqueJson object** (not a bare string):

```json
{
  "v": 1,
  "claims": {
    "iss": "…",
    "sub": "…",
    "aud": "…",
    "capabilities": ["…"],
    "exp": 1790000000
  },
  "sig": "<base64url-no-pad Ed25519 over JCS(claims)>"
}
```

| Field | Req | Semantics |
|-------|-----|-----------|
| `v` | yes | Token format version; protocol_version 1 uses **`1`** |
| `claims` | yes | Signed claims object (table above) |
| `sig` | yes | base64url (no padding) of the 64 raw Ed25519 signature bytes over JCS(`claims`) only |

**Canonical encoding (normative):** the `sig` field MUST be the unique RFC 4648 canonical base64url (no padding) encoding of the 64 raw signature bytes. Receivers MUST reject any `sig` that does not round-trip through `decode → encode` equality (non-canonical encodings of the final character's slack bits are rejected). Both language implementations enforce this at verify time.

Issuance MUST bind the issuer key to `claims.iss`: the Ed25519 public key used to sign MUST derive `claims.iss` per §[Identity binding](#identity-binding).

#### Trust root and validation rules

| Check | Requirement |
|-------|-------------|
| Trust root | Deployment-configured **`trusted_issuers`** list of issuer `peer_id` strings (parallel to the `noise-peerid` allowlist). **Empty list ⇒ method disabled**: challenges for `capability-token` MUST NOT be offered; proofs MUST be rejected. |
| Signature | Recompute JCS over `claims`; verify `sig` with the public key that derives `claims.iss`. |
| Issuer | After sig verify: `claims.iss` MUST be an exact-string member of `trusted_issuers`. |
| Subject | `claims.sub` MUST equal the authenticated session peer’s `peer_id` (the peer that passed `noise-peerid` hello). Tokens are not transferable across peers. |
| Audience | `claims.aud` MUST equal **this node’s** `peer_id` string. |
| Expiry / iat | `exp` required; reject expired. Apply `iat` and clock-skew rules when `iat` is present. |
| Malformed proof | Missing required wrapper/claim fields, wrong `v`, non-object `proof`, or unknown claim keys → reject. |

#### Capability matching

For an invoke, let `required = required_capability(op)` from the core table in §[Op dispatch gate](#op-dispatch-gate) (or product-documented `op` → capability mapping).

The token authorizes the op iff **`required` ∈ `claims.capabilities`** (membership / subset-of-grant). Extra capabilities on the token are ignored when unused. Matching is **not** exact-list equality between `claims.capabilities` and `negotiated_capabilities`.

The token does **not** replace `negotiated_capabilities`. Dispatch order when the method is in use:

1. Sequence / correlation checks (§[Session-core state machine](#session-core-state-machine))
2. **Optional capability-token gate** when policy requires a valid token (config or present `auth`)
3. `dispatch_allowed(op, negotiated_capabilities)` (§[Op dispatch gate](#op-dispatch-gate))
4. Handler

Both the token grant and the negotiated set MUST allow the op when the token gate is active.

#### Challenge / response and invoke `auth`

| Item | Rule |
|------|------|
| Hello | Unchanged `noise-peerid`. capability-token is step-up, not a hello replacement. |
| Config | Reference deployments expose `require_capability_token` (bool, default **`false`**) beside `trusted_issuers`. |
| When server challenges | If `trusted_issuers` is non-empty **and** `require_capability_token == true`: after both hellos are accepted (session would become Established), the server sends `ConnectAuthChallenge` before treating the session as fully authorized for invokes. If `require_capability_token == false`: no automatic challenge; token is validated when the client attaches invoke `auth` (and when a product defines additional per-op policy). |
| Challenge envelope | `method: "capability-token"`; `challenge_id` fresh unique string; `challenge` = opaque random nonce string (minLength ≥ 16). The server binds the nonce to `challenge_id` in a pending map (anti-replay for the challenge slot). Protocol_version 1 does **not** require the client to embed challenge bytes inside token claims. |
| Response | `ConnectAuthResponse { challenge_id, method: "capability-token", proof: <wrapper above>, extensions }`. Server validates `proof` per the rules above; on success the session records capability-token authorization (`capability_token_ok`); on failure the session remains unauthorized for invokes when the require flag is set. |
| Session grant | The challenge-validated grant is held for the **session lifetime**: invokes authorized by it are not expiry-re-checked per invoke. Products needing mid-session expiry enforcement implement their own re-challenge flow or attach a per-invoke `auth` proof (which is re-validated on every invoke). |
| Invoke `auth` | Optional `ConnectInvokeRequest.auth` carries the **same** proof object. When present, the receiver validates it on **every** invoke (expiry still valid; same subject/audience/issuer rules). When `require_capability_token` is true, the session is not token-ok, and `auth` is absent → reject the invoke. |
| Require flag false | When `require_capability_token` is false and no `auth` is attached, behavior matches `noise-peerid` alone (hello allowlist + signature). |

#### Error mapping

`ErrorEnvelope.code` remains an open string. No new error-envelope schema properties. Documented connect auth vocabulary:

| Condition | `code` |
|-----------|--------|
| Missing or invalid token when required; signature / issuer / audience / subject / expiry / malformed proof failure; unknown `method` on an auth response | `auth_failed` |
| Token valid but capability not granted for the requested `op` | `op_unsupported` (same path as dispatch-deny when the required capability is absent from the effective grant) |

Implementations MAY distinguish failure reasons in `message` and optional `details`; the machine code stays as above.

#### Non-goals (capability-token)

- Revocation lists / CRL / online status checks
- Refresh tokens or sliding expiry protocols
- A protocol-level token issuance HTTP (or RPC) endpoint
- Using `jti` for uniqueness enforcement in protocol_version 1 (field reserved only)
- Replacing `noise-peerid` hello identity with a token-only handshake

## Discovery boundary

- Connect **wire** has no mDNS/DHT/multiaddr fields.
- Normative discovery path: **explicit peering** (configured addresses / out-of-band dial).
- mDNS is a **runtime convenience** (non-default feature of the reference stack) for same-LAN development only — it is not implied as production discovery.

## Transport framing

Minimal framing contract for any transport that carries connect envelopes. Delimiter choice, retry, timeout, payload size limits, and flow control remain transport-adapter-owned (see §[Hard boundaries](#hard-boundaries)).

| Rule | Normative requirement |
|------|------------------------|
| Unit of message | **One JSON document** = one connect envelope (`ConnectHello`, invoke request/response, auth challenge/response, or optional `ConnectSession` snapshot). Protocol_version 1 does not batch multiple envelopes in one framing unit. |
| Stream assumption | Transport MUST present an **ordered, reliable, bidirectional byte stream**, or an equivalent request-response channel that preserves per-direction order for that session’s invokes. TCP, Yamux streams, WebSocket (one message frame per envelope), and libp2p request-response (one payload per RPC) are conforming when they meet this. |
| Delimiting | How JSON documents are delimited on the byte stream (length-prefix, newline, WebSocket one-message-per-envelope, one-payload-per-RPC) is **transport-adapter-owned**. The wire contract is the JSON document shape, not the delimiter. |
| Correlation | Application correlation uses existing fields only: response MUST echo `request_id` and `sequence` (and `session_id` per invoke tables). Framing adds no parallel correlation header. |
| Channel layout | The reference stack uses distinct protocol names for hello vs invoke; other stacks MAY multiplex on one stream when envelope types remain distinguishable by JSON shape. |

## Reference stack

The Rust reference maps these envelopes onto rust-libp2p: **noise** for authenticated transport, **yamux** for stream multiplexing, **request-response** for invoke, **identify** for peer metadata. Identity, session rules, and framing follow §[Identity binding](#identity-binding), §[Session-core state machine](#session-core-state-machine), and §[Transport framing](#transport-framing) — the same rules Path A (language-native) stacks implement in other languages. The crate `crates/spoke-connect` also exports the Path B uniffi binding surface; the libp2p stack is a transport demonstration, not a protocol surface. Kademlia / DHT discovery is not part of this protocol version.

## Hard boundaries

| Excluded concern | Where it lives |
|------------------|----------------|
| Daemon / shared runtime | Protocol repo is wire + spec only; runtimes are product-owned (the published reference crate `crates/spoke-connect` demonstrates transports and the binding surface, not a product runtime) |
| Ranking / retrieval / routing scores | Product-local |
| DHT / NAT traversal (Kademlia, Gossipsub, circuit relay) | Not in this protocol version; wire carries no multiaddr/DHT fields |
| Op semantics (upsert/check/assemble fields) | Ops wire (`schemas/ops/`) — invoke `payload` is opaque |
| Session / nonce / dispatch persistence | Adapter-owned — wire defines envelope shapes and ordering rules, not storage |
| Invoke timeout / retry / duplicate detection | Adapter-owned; `request_id` is the correlation handle for retries; protocol v1 defines no retry semantics |
| Payload size / flow control | Wire imposes no payload size limit; size bounding and flow control are transport-adapter-owned |
| Framing delimiter / multiaddr / HTTP status codes | Transport-adapter-owned (see §[Transport framing](#transport-framing)) |

**Design rules (hard):**

1. **Reuse, don't redefine** — hello embeds `HostCapabilityManifest` by `$ref`; invoke failure reuses `error-envelope`; payload uses `OpaqueJson`; identity binding is the libp2p Ed25519 PeerId formula in §[Identity binding](#identity-binding).
2. **Opaque payload wrapping** — invoke envelopes wrap existing ops envelopes as opaque JSON; connect does NOT re-specify upsert/check/assemble fields.
3. **Closed envelopes, open vocabulary** — `additionalProperties: false`; `op`, `method`, capability names stay open strings with documented core lists.
4. **`extensions` (ExtensionMap) required** on every connect envelope; hello-level `extensions` are unsigned.
5. **No engine fields** — no ranking, retrieval, routing scores, token budgets, multiaddrs, or DHT keys on connect wire.
6. **Dispatch gate** — op handlers run only when the required capability is in `negotiated_capabilities` (see §[Op dispatch gate](#op-dispatch-gate)).

## Acceptance (connect spec)

- [ ] Six connect envelopes committed under `schemas/connect/` — closed, required `extensions`
- [ ] Hello embeds `HostCapabilityManifest` via `$ref`; invoke-response error path reuses `error-envelope`
- [ ] Field tables in this spec match the schema files
- [ ] Signature algorithm named (`spoke-connect-hello-jcs-v1`, RFC 8785 JCS) with the exact signed field set
- [ ] Identity binding: protocol_version 1 Ed25519; `peer_id` = base58btc(identity multihash `0x00` of protobuf `PublicKey{Type=Ed25519, Data=32-byte pubkey}`)
- [ ] Ordering rules complete: start at 0, per-direction counters, echo correlation, no wrap-around
- [ ] Session-core state machine: Disconnected / Handshaking / Established / Closed with transition table
- [ ] Transport framing: one JSON document per message; ordered reliable stream; correlation via existing fields only
- [ ] Op dispatch gate MUST + core-op → capability mapping table
- [ ] Ops-family registration contract (capability + `op` vocabulary + opaque payload)
- [ ] Embedding model: three layers + Path A / Path B
- [ ] `spoke-connect` optional flag registered in `spoke-protocol-layers.md`; baseline unchanged
- [ ] Discovery default is explicit peering; mDNS described as non-default convenience only
- [ ] No transport-specific fields (multiaddr, DHT keys, HTTP/gRPC mapping) in connect schemas

## Non-goals (connect)

- Runtime / networking code (the published reference crate `crates/spoke-connect` ships its libp2p transport and uniffi binding surface alongside the session core; runtime placement stays product-owned)
- `did` auth method (reserved `method` name only)
- Capability-token revocation lists, refresh tokens, and protocol-level token issuance endpoints (offline validation only — see [§Method — capability-token](#method--capability-token))
- Kademlia / DHT discovery, Gossipsub, circuit relay / NAT traversal on the wire
- mDNS as a wire concept or default discovery mechanism
- TypeScript connectivity SDK or binding decisions
- Registry publish outside the `release.yml` stable-tag gate (all connect surfaces ship lockstep via the top-level release workflow — see [`connect-publish-strategy.md`](connect-publish-strategy.md))
- Changing baseline capability requirements or existing ops schemas
- Re-defining `HostCapabilityManifest` fields (embed by `$ref` only)
- Connect envelope shape changes for new ops families (registration uses vocabulary + capabilities + opaque payload)

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella framing and schema inventory |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8, capability levels, Optional flags (`spoke-connect`) |
| [`spoke-data-model.md`](spoke-data-model.md) | `HostCapabilityManifest` field tables (embedded by hello) |
| [`spoke-ops.md`](spoke-ops.md) | Ops wire + error envelope (wrapped as opaque invoke payloads) |
| [`spoke-operations.md`](spoke-operations.md) | Lifecycle helpers; adapter ports; `HostManifestPort` |
| [`schemas/README.md`](../../schemas/README.md) | Connect schema files under `schemas/connect/` |
| [`crates/spoke-connect/README.md`](../../crates/spoke-connect/README.md) | Rust reference (libp2p transport + Path B uniffi binding surface) |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Connect envelope family / Session (connect) / peer_id vocabulary |
| [`STRATEGY.md`](../../STRATEGY.md) | Protocol-not-runtime positioning |
