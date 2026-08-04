---
title: Connect overview
---

# Connect overview

Connect is the opt-in **interaction envelope family** (`spoke-connect` capability flag) for cross-process SPOKE hosts: signed manifest exchange, session context, remote op invocation, and extensible authentication. The family is additive — baseline compliance and baseline schemas stay unchanged.

## Six envelopes

- **ConnectHello** — signed manifest exchange (embeds `HostCapabilityManifest` by `$ref`).
- **ConnectSession** — established session context snapshot (peer ids, negotiated capabilities, initial sequence).
- **ConnectInvokeRequest / ConnectInvokeResponse** — remote op calls wrapping existing ops envelopes as opaque `payload`; failures reuse the shared `ErrorEnvelope`.
- **ConnectAuthChallenge / ConnectAuthResponse** — method-extensible auth (`method` open string).

## Design rules

- **Reuse** — hello embeds the data-layer manifest; invoke wraps ops envelopes; identity is the existing opaque `peer_id`.
- **Identity** — `peer_id` (trust root; libp2p Ed25519 PeerId in protocol v1) vs `host_id` (advisory label inside the manifest).
- **Signed hello** — `spoke-connect-hello-jcs-v1`: RFC 8785 JCS over `{protocol_version, peer_id, nonce, host}`, Ed25519-signed, unpadded base64url.
- **Ordering** — per-session, per-direction monotonic `sequence` from 0; a sequence overflow closes the session and opens a new one; responses echo `session_id` / `sequence` / `request_id`.
- **Auth** — `noise-peerid` (allowlist + signed hello at handshake) and `capability-token` (offline-validated, capability-scoped step-up grants).
- **Discovery** — explicit peering (configured addresses / out-of-band dial) is the normative production path; mDNS is a non-default reference-stack convenience for same-LAN development only.

## Embedding model

- **Language-native client** — implement wire + session-core rules in the host language (the [TypeScript route](/connect/ts-route)).
- **Native bindings** — embed a shared session core via FFI into host languages ([native bindings](/connect/bindings)).

## Transport

One JSON connect envelope per message over an ordered, reliable, bidirectional byte stream (TCP, WebSocket, yamux, libp2p request-response). Framing delimiters, retries, and payload limits are transport-adapter-owned. The Rust reference (`crates/spoke-connect`) ships libp2p transport plus a uniffi binding surface.

## Normative references

- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) — envelope field tables, identity binding, JCS, session-core state machine, auth model, discovery boundary, transport framing
- [schemas/connect/](https://github.com/42ch-dev/spoke/tree/main/schemas/connect) — committed connect schemas
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — connect vocabulary (peer_id, capability token, Session)
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) — `spoke-connect` capability flag
