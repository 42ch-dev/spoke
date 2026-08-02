---
title: TypeScript connect route
---

# TypeScript connect route

The TypeScript route decision picks a client stack for browser and Node integrators implementing connect Path A. The locked recommendation is **pure-TS-minimal** — the envelope rules and identity math are implemented in TypeScript without a libp2p or WASM dependency.

## Locked recommendation

- **Transport (first slice)** — WebSocket as an ordered reliable stream, one JSON connect envelope per message.
- **Crypto** — WebCrypto `Ed25519` where available, `@noble/ed25519` fallback (same seed/sign semantics).
- **Canonicalization** — RFC 8785 JCS over the signed hello fields.
- **`peer_id`** — the spec formula only: protobuf `PublicKey` → identity multihash `0x00` → base58btc (no multibase prefix, no CIDv1).

## Fallback routes

- **js-libp2p** — when the product must join a shared libp2p mesh and speak Noise/yamux with the Rust reference stack (mesh fallback, not the default).
- **WASM (Rust core + JS transport)** — deferred; only if a future pure-TS crypto/JCS gap appears.

## Evidence

Identity-byte reproducibility in JavaScript is proven: `tooling/connect-identity-proof/proof.mjs` (zero npm dependencies) matches the Rust golden vectors for `peer_id`, JCS bytes, Ed25519 signature, and base64url encoding — all checks PASS on Node 24 WebCrypto.

## First-slice scope (suggested)

The first slice is scoped to pure helpers (`derivePeerId`, canonical hello bytes / JCS, `signHello` / `verifyHello`, base64url), a WebSocket transport adapter, and a minimal session-core port (protocol version check, allowlist, nonce single-use, per-direction sequence, `request_id` correlation, dispatch gate); swarm features, DHT discovery, and a published npm connect package are later slices.

## Normative references

- [spoke-connect-ts-route.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect-ts-route.md) — full evaluation, rationale, overturn check, identity proof evidence
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) — normative wire / identity / framing (the route does not change envelopes)
- [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md) — publishing and packaging strategy for the TS surface
- [tooling/connect-identity-proof/](https://github.com/42ch-dev/spoke/tree/main/tooling/connect-identity-proof) — local JS reproducibility proof
