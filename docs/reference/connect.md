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
| `protocol_version` | Connect protocol version (distinct from data `schema_version`); version 1 is current |
| `peer_id` | Sender network identity — protocol v1: libp2p identity-spec PeerId string for Ed25519 (base58btc identity multihash of the protobuf `PublicKey`). Opaque to protocol logic; the trust root for the `noise-peerid` allowlist |
| `nonce` | Single-use replay nonce, bound into the signed object |
| `host` | Full embedded `HostCapabilityManifest` (includes `host.extensions`); part of the signed object |
| `signature` | base64url (no padding) of the raw signature bytes over the JCS-canonicalized signed object |
| `extensions` | Product bag; not covered by the signature |

### ConnectSession — established session context

Required: `session_id`, `initiator_peer_id`, `responder_peer_id`, `opened_at`, `negotiated_capabilities`, `initial_sequence`, `extensions`.

| Field | Notes |
|-------|-------|
| `session_id` | Opaque session id (UUID recommended; not schema-enforced) |
| `initiator_peer_id` / `responder_peer_id` | The peer that dialed / the peer that accepted |
| `opened_at` | Session open time (UTC) |
| `negotiated_capabilities` | Intersection (or agreed subset) of both hosts' `capabilities[]`; includes `spoke-connect` when both declare it |
| `initial_sequence` | First invoke request uses this sequence (0 for protocol version 1) |
| `extensions` | Product namespace bag |

### ConnectInvokeRequest / ConnectInvokeResponse — remote op calls

`ConnectInvokeRequest` required: `session_id`, `sequence`, `request_id`, `op`, `payload`, `extensions`.

| Field | Notes |
|-------|-------|
| `session_id` | Opaque session id |
| `sequence` | Monotonic per-session outbound from this sender; logical u64 capped at 2^53−1 (JSON-safe) |
| `request_id` | Caller-generated correlation id (UUID recommended) |
| `op` | Open vocabulary. Core list (documented, not enforced): `upsert`, `promote`, `relate`, `check`, `assemble`, `project`, `compute` |
| `payload` | Opaque JSON — a full existing ops request envelope for the named op when targeting SPOKE ops |
| `auth` | Optional mid-session proof blob; primary auth is the hello. Shape is method-specific when used |
| `extensions` | Product namespace bag |

`ConnectInvokeResponse` is the success `{ payload }` **or** `{ error }` — the same one-failure dialect as the ops wire; failures reuse the shared `ErrorEnvelope`.

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
2. The signed object is exactly `{protocol_version, peer_id, nonce, host}` — top-level `extensions` and `signature` are excluded.
3. The object is canonicalized with RFC 8785 JCS ([RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)).
4. The bytes are signed with the Ed25519 keypair; the raw signature is encoded base64url without padding ([RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648)).
5. The receiver accepts only when: protocol version is 1, the claimed `peer_id` equals the authenticated remote peer, the key derives that peer id, the peer is on the configured allowlist (empty allowlist rejects all — fail-closed), the signature verifies, and the `(peer_id, nonce)` pair is new (single-use, process lifetime).

## Ordering and correlation

Per-session, per-direction monotonic `sequence` counters start at 0; a sequence overflow closes the session and opens a new one. Invoke responses echo `session_id` / `sequence` / `request_id` — the correlation check fails on any mismatch. The receiver enforces inbound sequence monotonicity and answers replayed or out-of-order sequences with an `invalid_sequence` wire envelope.

## Auth methods

| Method | How it works |
|--------|--------------|
| `noise-peerid` | The handshake default: allowlist admission plus the signed hello, with the remote peer authenticated by the transport (noise) |
| `capability-token` | Step-up / mid-session grant: a trusted issuer signs a short claim set (`iss` / `sub` / `aud` / `capabilities` / `exp`, optional `iat` / `jti`) over JCS with Ed25519; the proof rides the challenge/response exchange or per-invoke `auth`. Verification enforces issuer trust, subject/audience binding, expiry, and clock skew. An empty trusted-issuer list disables the method (fail-closed) |

## Discovery and peering

**Explicit peering is the production path**: hosts are configured with listen addresses and dial each other out-of-band (configured addresses or direct dial). The connect wire carries no discovery fields — mDNS is a same-LAN development convenience offered by the Rust reference stack's optional `mdns` feature, and discovered candidates are admitted through the same allowlist and signed-hello gates as explicitly dialed peers.

## Transport

One JSON connect envelope per message over an ordered, reliable, bidirectional byte stream (TCP, WebSocket, yamux, libp2p request-response). Framing delimiters, retries, and payload limits are transport-adapter-owned.

## Embedding model

| Path | What ships |
|------|-----------|
| **Language-native client** | The wire contract and session-core rules implemented in the host language (the TypeScript `@42ch/spoke-connect` client with WebSocket transport) |
| **Native bindings** | The shared session core exported into host languages via FFI (C# NuGet, Kotlin Maven, Swift SPM, Go modules, Python PyPI) |
| **Rust reference** | The published `spoke-connect` crate: session-core reference, uniffi binding source, and a rust-libp2p transport stack ([crate README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md)) |

The session-core rules — allowlist, `peer_id` derive and reverse, hello crypto, nonce, request correlation, sequence, capability-token auth, and the dispatch gate — are shared across every language and locked by golden vectors. Thin client conveniences (`Session`, `negotiatedCapabilities`, `generateNonce`) are provided where the host runtime benefits from them.

## Related

- [Open your first connect session](/tutorials/first-connect-session) — the flow end to end.
- [Connect from the TypeScript client](/how-to/connect-ts-client) — the language-native client surface.
- [Connect from native bindings](/how-to/connect-native-bindings) — the FFI bindings with install pins.
- [Protocol reference](/reference/protocol) — the `spoke-connect` capability flag.
