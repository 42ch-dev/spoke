---
title: Connect reference
---

# Connect reference

Connect is the opt-in **interaction envelope family** (`spoke-connect` capability flag) for cross-process SPOKE hosts: signed manifest exchange, session context, remote op invocation, and extensible authentication. The family is additive — baseline compliance and baseline schemas stay unchanged. Field tables below trace to the committed schemas in [`schemas/connect/`](https://github.com/42ch-dev/spoke/tree/main/schemas/connect).

## The six envelopes

### ConnectHello — signed manifest exchange

Required: `protocol_version`, `peer_id`, `nonce`, `host`, `signature`, `extensions`.

| Field | Notes |
|-------|-------|
| `protocol_version` | The `protocol_version` value consumers set in a `ConnectHello` is **1** (the hello exchange has not been bumped; the hello signed-field set and `spoke-connect-hello-jcs-v1` algorithm are unchanged). Protocol version **2 is current** as the normative connect-protocol version: it adds required per-envelope signatures on the **post-hello** wire, enforced internally by the RemoteAdapter and never set by consumers as a field value (see [Envelope authentication (protocol_version 2)](#envelope-authentication-protocol_version-2)). Bindings' `protocolVersion()` reports the hello version (1). |
| `peer_id` | Sender network identity — protocol v1: libp2p identity-spec PeerId string for Ed25519 (base58btc identity multihash of the protobuf `PublicKey`). Opaque to protocol logic; the trust root for the `noise-peerid` allowlist |
| `nonce` | Single-use replay nonce, bound into the signed object |
| `peer_nonce` | Responder-only dial binding: the initiator's nonce, echoed by the responder and bound into its signed object. Absent in initiator hellos; initiators reject a responder hello whose `peer_nonce` differs from their own nonce |
| `host` | Full embedded `HostCapabilityManifest` (includes `host.extensions`); part of the signed object |
| `signature` | base64url (no padding) of the raw signature bytes over the JCS-canonicalized signed object |
| `extensions` | Product bag; not covered by the signature |

### ConnectSession — established session context

Required: `session_id`, `initiator_peer_id`, `responder_peer_id`, `opened_at`, `negotiated_capabilities`, `initial_sequence`, `extensions` (+ `signature` in protocol_version 2).

| Field | Notes |
|-------|-------|
| `session_id` | Opaque session id (UUID recommended; not schema-enforced) |
| `initiator_peer_id` / `responder_peer_id` | The peer that dialed / the peer that accepted; must equal the authenticated hello `peer_id` values |
| `opened_at` | Session open time (UTC) |
| `negotiated_capabilities` | Intersection (or agreed subset) of both hosts' `capabilities[]`; includes `spoke-connect` when both declare it |
| `initial_sequence` | First invoke request uses this sequence — const 0 for protocol versions 1 and 2 |
| `signature` | v2 only, required, minLength 86 maxLength 86 — base64url (no padding) of the 64-byte Ed25519 signature over the JCS-canonicalized signed object (`spoke-connect-session-jcs-v1`); see [Envelope authentication (protocol_version 2)](#envelope-authentication-protocol_version-2) |
| `extensions` | Product namespace bag |

### ConnectInvokeRequest / ConnectInvokeResponse — remote op calls

`ConnectInvokeRequest` required: `session_id`, `sequence`, `request_id`, `op`, `payload`, `extensions` (+ `signature` in protocol_version 2).

| Field | Notes |
|-------|-------|
| `session_id` | Opaque session id |
| `sequence` | Monotonic per-session outbound from this sender; logical u64 capped at 2^53−1 (JSON-safe) |
| `request_id` | Caller-generated correlation id (UUID recommended) |
| `op` | Open vocabulary. Core list (documented, not enforced): `upsert`, `promote`, `relate`, `check`, `assemble`, `project`, `compute`; reserved `port.*` prefix for RemoteAdapter port methods (see [Port-method ops (RemoteAdapter)](#port-method-ops-remoteadapter)) |
| `payload` | Opaque JSON — a full existing ops request envelope for the named op when targeting SPOKE ops |
| `auth` | Optional mid-session proof blob; primary auth is the hello. Shape is method-specific when used. When present on protocol_version 2 wire, `auth` is included in the JCS signed object |
| `signature` | v2 only, required, minLength 86 maxLength 86 — base64url (no padding) of the 64-byte Ed25519 signature over the JCS-canonicalized signed object (`spoke-connect-invoke-request-jcs-v1`); see [Envelope authentication (protocol_version 2)](#envelope-authentication-protocol_version-2) |
| `extensions` | Product namespace bag |

`ConnectInvokeResponse` is the success `{ payload }` **or** `{ error }` — the same one-failure dialect as the ops wire; failures reuse the shared `ErrorEnvelope`. Both branches add a required `signature` in protocol_version 2:

| Branch | v2 `signature` |
|--------|----------------|
| Success `{ session_id, sequence, request_id, payload, extensions }` | Required, minLength 86 maxLength 86 — `spoke-connect-invoke-response-jcs-v1` over `{session_id, sequence, request_id, payload}` |
| Error `{ session_id, sequence, request_id, error, extensions }` | Required, minLength 86 maxLength 86 — `spoke-connect-invoke-response-jcs-v1` over `{session_id, sequence, request_id, error}` |

### ConnectAuthChallenge / ConnectAuthResponse — extensible auth

`ConnectAuthChallenge` required: `challenge_id`, `method`, `challenge`, `extensions`. `ConnectAuthResponse` required: `challenge_id`, `method`, `proof`, `extensions`.

| Field | Notes |
|-------|-------|
| `challenge_id` | Correlation id echoed by the response |
| `method` | Open vocabulary. Core list (documented, not enforced): `noise-peerid`, `capability-token`; reserved name: `did` |
| `challenge` / `proof` | Opaque method-specific material; for `capability-token`, `proof` is `{ v, claims, sig }` with `sig` over JCS(`claims`) only |

## Identity

`peer_id` is the network trust root: the libp2p identity-spec PeerId (Ed25519, base58btc identity multihash). `host_id` is an advisory application label inside the embedded `HostCapabilityManifest`. The two stay distinct — one host may present multiple peer ids over time, and one peer id derives from exactly one Ed25519 public key.

## Signed hello (`spoke-connect-hello-jcs-v1`)

1. Both sides of a connection exchange a signed `ConnectHello`.
2. The signed object is `{protocol_version, peer_id, nonce, host}` for the **initiator** hello (4 fields — `peer_nonce` absent), and `{protocol_version, peer_id, nonce, host, peer_nonce}` for the **responder** hello (5 fields — `peer_nonce` = the initiator's nonce, dial binding). Top-level `extensions` and `signature` are excluded.
3. The object is canonicalized with RFC 8785 JCS ([RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)).
4. The bytes are signed with the Ed25519 keypair; the raw signature is encoded base64url without padding ([RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648)).
5. The receiver accepts only when: protocol version is 1, the claimed `peer_id` equals the authenticated remote peer, the key derives that peer id, the peer is on the configured allowlist (empty allowlist rejects all — fail-closed), the signature verifies over the role-aware field set, and the `(peer_id, nonce)` pair is new (single-use, process lifetime).
6. Dial binding: the initiator additionally requires the responder's signed `peer_nonce` to equal its own nonce — a captured responder hello cannot be replayed into a fresh dial (e.g. after a client restart resets the in-memory nonce store).

## Envelope authentication (protocol_version 2)

Protocol version **2** authenticates every post-hello trust-affecting envelope — `ConnectSession`, `ConnectInvokeRequest`, `ConnectInvokeResponse` — at the protocol layer using per-envelope JCS + Ed25519 signed-field sets, the same construction as `spoke-connect-hello-jcs-v1` extended to three new algorithm ids. Receivers verify envelope authenticity at the protocol layer, independent of transport-level TLS or Noise; a transport-supplied authenticated peer identity does not relax the rules.

### Algorithm ids

| Algorithm id | Envelope |
|--------------|----------|
| `spoke-connect-hello-jcs-v1` | `ConnectHello` (unchanged) |
| `spoke-connect-session-jcs-v1` | `ConnectSession` |
| `spoke-connect-invoke-request-jcs-v1` | `ConnectInvokeRequest` |
| `spoke-connect-invoke-response-jcs-v1` | `ConnectInvokeResponse` |

All four share the construction: RFC 8785 JCS → UTF-8 bytes → Ed25519 sign/verify → base64url without padding. The signing key is the sender's peer-identity Ed25519 private key; verification uses the public key that derives the authenticated hello `peer_id`.

### Authenticated field sets

Each signed object is a strict subset of the wire envelope (exact keys, no others). `extensions` and `signature` are excluded — `extensions` are not covered by the signature and stay outside trust decisions:

- **`ConnectSession`**: `{session_id, initiator_peer_id, responder_peer_id, opened_at, negotiated_capabilities, initial_sequence}`
- **`ConnectInvokeRequest`**: `{session_id, sequence, request_id, op, payload}` and additionally `auth` **when present** on the wire
- **`ConnectInvokeResponse`** success branch: `{session_id, sequence, request_id, payload}`
- **`ConnectInvokeResponse`** error branch: `{session_id, sequence, request_id, error}`

The two response branches are signed over their respective field sets; the `signature` field must be the canonical base64url (no padding) encoding of the 64 raw signature bytes.

### Verify rules

For every v2 post-hello envelope the receiver: (1) presence-checks `signature`; (2) runs the canonical-encoding round-trip check; (3) builds the signed object per the locked field set; (4) canonicalizes with RFC 8785 JCS; (5) verifies with the peer's hello Ed25519 public key; (6) asserts the session binding (`session_id` bound to an established session, peer ids matching the authenticated hellos). Any failure rejects the envelope. Signatures bind to the session: a `session_id` not bound to an established session rejects, and an envelope captured from one session replayed into another fails verification against the other session's hello key.

### Version strategy

| Direction | Behavior |
|-----------|----------|
| v2 peer ↔ v1 peer | The v2 side observes `protocol_version: 1` in the verified hello and refuses to establish — the v1 side cannot produce signed session/invoke envelopes. Dial fails closed |
| Both v2 | Session establishes under v2 rules; all post-hello envelopes carry the required `signature` |
| Both v1 | Legacy v1 interop only |
| Unknown version (> 2) | A verified hello advertising an unknown version fails closed, treated like a mixed-version dial |

The hello signed-field set (4-field initiator / 5-field responder) is unchanged; the dial-binding `peer_nonce` rule is preserved.

### Error mapping

Envelope-auth failures use the shared `ErrorEnvelope` vocabulary: `auth_failed` covers missing, invalid, non-canonical, or field-set-drifted signatures and session-binding mismatches. On the RemoteAdapter surface these surface as `SpokeResult` rejects — `INTERNAL_ERROR` with `details.kind` ∈ {`envelope_auth_missing`, `envelope_auth_invalid`, `envelope_auth_session_unbound`} — while a mixed- or unknown-version hello fails the dial as a handshake error (`RemoteAdapterError::Handshake`; no adapter instance). `protocol_version_mismatch` is spec vocabulary reserved for a dedicated version-negotiation reject and is not currently emitted.

### Enforcement

The RemoteAdapter (`./remote` subpath in TypeScript, `remote-adapter` feature in Rust) and the connect-client enforce v2 per-envelope authentication internally on every post-hello envelope they emit or accept, with nothing to configure: the dial verifies the signed `ConnectSession` snapshot at establish, every outbound invoke request is signed, and every correlated response is verified after the correlation echo check. The hello exchange remains at protocol version 1. `ConnectAuthChallenge` / `ConnectAuthResponse` carry method-specific proofs that are already signature-bound; they are outside the v2 per-envelope signing.

## Ordering and correlation

Per-session, per-direction monotonic `sequence` counters start at 0; a sequence overflow closes the session and opens a new one. Invoke responses echo `session_id` / `sequence` / `request_id` — the correlation check fails on any mismatch. The receiver enforces inbound sequence monotonicity and answers replayed or out-of-order sequences with an `invalid_sequence` wire envelope.

## Session-core state machine

The session core tracks one logical state per local node per session:

| State | Meaning |
|-------|---------|
| `Disconnected` | No transport session; no outbound sequence for this session |
| `Handshaking` | Transport up; hellos in flight; invokes not yet authorized |
| `Established` | Both hellos accepted; `session_id` assigned; outbound counter = 0; inbound expected = 0 |
| `Closed` | Session unusable (sequence exhaustion, transport loss, auth failure, local shutdown); open a new session — no sequence wrap |

| Transition | Trigger | Guards / effects |
|------------|---------|------------------|
| `Disconnected` → `Handshaking` | Transport connect/accept | — |
| `Handshaking` → `Established` | Local accept of remote hello and remote accept of local hello | Allowlist + signature + nonce single-use; dial binding (responder's signed `peer_nonce` = initiator's nonce); session peer ids bound to the authenticated hello `peer_id`s; `negotiated_capabilities` = agreed subset; outbound counter = 0 and inbound expected = 0 |
| `Handshaking` → `Closed` | Any hello gate failure | Nonce of a rejected hello is not recorded |
| `Established` → `Established` | Outbound invoke | Atomically allocate `sequence = last + 1` starting at 0; attach a new `request_id`; send |
| `Established` → `Established` | Inbound invoke | Accept iff `sequence == next_expected_inbound` (start 0), then advance; else reject with a wire error and no handler side effect |
| `Established` → `Established` | Inbound response | Accept iff it echoes `session_id`, `sequence`, and `request_id` of a pending request; else correlation failure |
| `Established` → `Closed` | Next outbound sequence would exceed 2^53−1, transport loss, or local shutdown | No wrap-around |

On a v2 wire, sequence/correlation checks run first but do not advance session state until envelope-auth verification passes (see [Envelope authentication (protocol_version 2)](#envelope-authentication-protocol_version-2)).

## Auth methods

| Method | How it works |
|--------|--------------|
| `noise-peerid` | The handshake default: allowlist admission plus the signed hello, with the remote peer authenticated by the transport (noise) |
| `capability-token` | Step-up / mid-session grant: a trusted issuer signs a short claim set (`iss` / `sub` / `aud` / `capabilities` / `exp`, optional `iat` / `jti`) over JCS with Ed25519; the proof rides the challenge/response exchange or per-invoke `auth`. Verification enforces issuer trust, subject/audience binding, expiry, and clock skew. An empty trusted-issuer list disables the method (fail-closed) |

## Capability vocabulary

Each operation maps to the capability it requires on the session's `negotiated_capabilities` (the dispatch gate evaluates the negotiated set, not the remote manifest alone):

| Operation | Required capability |
|-----------|---------------------|
| `upsert`, `promote`, `relate`, `check`, `assemble` (and the `port.*` baseline ops) | `spoke-baseline` |
| `project`, `compute` (and `port.computable.*`) | `l2-computable` |
| Product-defined operations | The capability the product documents |

A capability-token grant authorizes session membership for the ops its `capabilities[]` covers, but it does not replace `negotiated_capabilities` — both the token grant and the negotiated set must allow an op when the token gate is active.

## Port-method ops (RemoteAdapter)

The RemoteAdapter proxies each `BaselinePorts` method as a connect invoke with a reserved `port.*` product op and an opaque snake_case payload:

| Method | `op` | Request `payload` | Success `payload` |
|--------|------|-------------------|-------------------|
| `getKnowledgeEntry` | `port.knowledge.get` | `{ "entry_id": string }` | `KnowledgeEntry` |
| `putKnowledgeEntry` | `port.knowledge.put` | `{ "entry": KnowledgeEntry, "expected_base_revision": number \| null }` | `KnowledgeEntry` |
| `getRelation` | `port.relation.get` | `{ "relation_id": string }` | `Relation` |
| `putRelation` | `port.relation.put` | `{ "relation": Relation, "expected_base_revision": number \| null }` | `Relation` |
| `listKnowledgeEntries` | `port.scope.list_knowledge_entries` | `{ "scope": Scope }` | `KnowledgeEntry[]` |
| `listTimelineEvents` | `port.scope.list_timeline_events` | `{ "scope": Scope }` | `TimelineEvent[]` |
| `putFindings` | `port.finding.put` | `{ "findings": Finding[] }` | `Finding[]` |
| `listRules` | `port.rule.list` | `{ "rule_refs": string[] }` | `Rule[]` |
| `listPeerHostCapabilityManifests` | `port.host.list_peer_manifests` | `{}` | `HostCapabilityManifest[]` |
| `getHostCapabilityManifest` | *(none — session cache)* | — | The remote hello `host`, cached at establish; no round-trip |

Optional families reserve `port.computable.*` (`project` / `compute`, `l2-computable`) and `port.fork.*` (`listForkTimelineEvents`, `l5-fork`) for future products; the baseline adapter ships the table above.

## Discovery and peering

**Explicit peering is the production path**: hosts are configured with listen addresses and dial each other out-of-band (configured addresses or direct dial). The connect wire carries no discovery fields — mDNS is a same-LAN development convenience offered by the Rust reference stack's optional `mdns` feature, and discovered candidates are admitted through the same allowlist and signed-hello gates as explicitly dialed peers.

## Transport

One JSON connect envelope per message over an ordered, reliable, bidirectional byte stream (TCP, WebSocket, yamux, libp2p request-response). Framing delimiters, retries, and payload limits are transport-adapter-owned.

## Embedding model

| Embedding | What ships |
|-----------|------------|
| **Language-native client** | The wire contract and session-core rules implemented in the host language (the TypeScript `@42ch/spoke-connect` client with WebSocket transport) |
| **Native bindings** | The shared session core exported into host languages via FFI (C# NuGet, Kotlin Maven, Swift SPM, Go modules, Python PyPI) |
| **Rust reference** | The published `spoke-connect` crate: session-core reference, uniffi binding source, and a rust-libp2p transport stack ([crate README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md)) |

The session-core rules — allowlist, `peer_id` derive and reverse, hello crypto, nonce, request correlation, sequence, capability-token auth, and the dispatch gate — are shared across every language and locked by golden vectors. Thin client conveniences (`Session`, `negotiatedCapabilities`, `generateNonce`) are provided where the host runtime benefits from them.

## Error vocabulary

| Entry | Where it appears | Meaning |
|-------|------------------|---------|
| `auth_failed` | `ErrorEnvelope.code` | Token auth failures (missing or invalid token when required; signature / issuer / audience / subject / expiry / malformed proof) and all envelope-auth failures (missing, invalid, or non-canonical signature; field-set drift; session-binding mismatch) |
| `invalid_sequence` | `ErrorEnvelope.code` | Replayed or out-of-order inbound sequence |
| `op_unsupported` | `ErrorEnvelope.code` | Unknown `op`, or a token valid but without the capability for the requested op |
| `capability_missing` | `ErrorEnvelope.code` | The op's required capability is absent from the effective grant |
| `no_capable_peer` | Router reject `details.wire_code` / `details.kind` | Terminal router reject when no registered peer passes the hard selection gates (`CAPABILITY_PORT_MISSING`); register a satisfying peer and re-invoke |
| `envelope_auth_missing` / `envelope_auth_invalid` / `envelope_auth_session_unbound` | RemoteAdapter reject `details.kind` | Envelope-auth rejection kinds on `INTERNAL_ERROR` rejects (waiter only; session state untouched) |
| `handshake` | Dial failure `details.kind` (`FfiError.Dial`) | A verified hello advertising a mixed or unknown protocol version fails the dial as a handshake error; the dial surface is exactly {`config`, `handshake`, `timeout`} and no adapter instance exists |
| `protocol_version_mismatch` | Reserved — not emitted | Spec vocabulary reserved for a dedicated version-negotiation reject; mixed/unknown-version hellos fail as a `handshake` dial error |
| `transport` / `session_closed` / `timeout` / `panic` / `correlation_mismatch` / `sequence_exhausted` | RemoteAdapter reject `details.kind` | `INTERNAL_ERROR` reject kinds for transport I/O, session loss, invoke timeout, panic containment, correlation mismatch, and sequence exhaustion |

## Related

- [Open your first connect session](/tutorials/first-connect-session) — the flow end to end.
- [Connect from the TypeScript client](/how-to/connect-ts-client) — the language-native client surface.
- [RemoteAdapter over a Transport](/how-to/connect-remote-adapter) — dial a remote peer over a consumer `Transport` and call its `BaselinePorts` surface.
- [Route across multiple peers](/how-to/multi-peer-routing) — the router recipe over N registered adapters.
- [Connect from native bindings](/how-to/connect-native-bindings) — the FFI bindings with install pins.
- [Connect architecture](/explanation/connect) — session lifecycle, envelope authentication, and capability routing.
- [Protocol reference](/reference/protocol) — the `spoke-connect` capability flag.
