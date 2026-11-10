# ADR: Pure TypeScript as the primary connect client stack

Status: implemented — the TypeScript client is a pure-TS port of the wire and session core, with js-libp2p kept as an explicit mesh fallback

Date: 2026-08-01

Scope: TypeScript connectivity route for browser and Node integrators — stack evaluation, the chosen route, and the fallback paths

## Problem

A TypeScript client must produce exactly the same signed hello bytes and the same `peer_id` derivation as the Rust reference, otherwise envelope-level interop fails. Several stacks could reach that byte parity, and they differ in transport fit, dependency weight, session-core cost and whether they force a Rust or WASM toolchain into a protocol repository:

- **js-libp2p** brings transports and libp2p identity primitives, but SPOKE's hello is JCS + Ed25519 over SPOKE's own field set, not a libp2p native hello, and the session rules still need a port.
- **WASM** (Rust session core compiled to WASM with a JS transport) gives byte parity by construction, but still needs a JS transport adapter and adds a wasm-pack / wasm-bindgen CI and artifact policy.
- **pure TypeScript** needs the identity math implemented in JS — the open question was whether WebCrypto and a JCS implementation can reproduce Rust's canonical bytes.

Selection was scored against ten criteria: browser transport, Node transport, identity/PeerId, signing, RFC 8785 JCS, session-core port cost, dependency weight, ecosystem maintenance, fit with the embedding model, and interop with the Rust reference. Scores were fitness judgements, not a summed total.

## Decision

The primary TypeScript route is **pure-TS-minimal**: implement the wire contract and the session-core rules in TypeScript and pair them with a plain ordered-stream transport.

- Transport for the first slice: WebSocket carrying one JSON connect envelope per message.
- Crypto: WebCrypto `Ed25519` where available, `@noble/ed25519` as the fallback with the same seed and signing semantics.
- Canonicalization: RFC 8785 JCS over the role-aware hello field set, either implemented directly or backed by a maintained JCS library proven against the golden vectors.
- `peer_id`: the spec formula only — protobuf `PublicKey` → identity multihash `0x00` → base58btc, with no multibase prefix and no CIDv1.
- Session core: a small pure port of sequence, nonce, allowlist, correlation and the dispatch gate, with I/O staying at the adapter boundary.

The route is accepted because byte reproducibility in JavaScript was **proven**, not assumed: an identity proof (zero npm dependencies) reproduces the golden `peer_id`, the golden JCS UTF-8 bytes, the 64-byte Ed25519 signature and its base64url encoding, and verifies the golden signature. That proof runs as a required CI check, so the pure-TS route cannot silently drift from the Rust reference.

Two constraints follow from choosing JS as the porting language: absent optional host fields must be omitted rather than serialized as `null` (emitting `null` changes the canonical bytes), and the parity surface is limited to the shared *rules* — thin client conveniences such as a session wrapper, nonce generation and correlation helpers stay outside parity because Rust keeps equivalent state in its transport layer.

## Alternatives considered

**js-libp2p as the primary client stack.** Strong on transport (WebSocket and WebRTC with ordered streams), strong on identity primitives, and the natural choice when a product must dial and listen on the same Noise/yamux mesh as the Rust reference. It lost as the *default* for three reasons: the `@libp2p/*` monorepo is a deep supply-chain and bundle surface for what should be a thin helper package; SPOKE still owns the JCS hello and the session core outside the mesh, so the port cost remains; and a mesh-first client nudges integrators toward reading connect as a libp2p network rather than as envelopes over an ordered stream. It stays the documented fallback when a product needs shared-mesh membership, available as an optional companion package or a consumer-repo adapter rather than a forced default.

**WASM (Rust session core plus a JS transport).** Best byte parity and no session-core port at all, and it reuses the Rust `peer_id` and hello signing directly. It lost because it does not remove the need for a JS transport adapter, it adds wasm-pack / wasm-bindgen CI and binary artifact policy to a protocol repository that ships no connect runtime, and it fails hosts that decline WASM — which would leave browsers needing the pure path anyway. It stays a contingency, and the recorded trigger is a pure-TS crypto or JCS gap that WebCrypto and `@noble` cannot close.

**Forking the signed-field sets for TypeScript convenience.** Rejected outright: if pure-TS JCS diverges on a new host field shape, the fix is the TS JCS path or the golden vectors, never a divergent signed-field set. Byte parity is the whole point of the route.

## Consequences

- The identity proof is a required, deliberately unfiltered CI check; a changed-path gate would report green without running it.
- The client ships as a plain package with WebCrypto plus one optional curve dependency — no swarm, no mDNS, no DHT in the default export graph.
- The parity surface is explicit and testable: peer-id reverse-derivation length cap, canonical base64url signature encoding, capability-token issuance ergonomics, and verify-time token rules, each proven by shared golden vectors in both directions.
- A Noise option can be added later as a transport-adapter-owned subpath with subpath-only dependencies; the default bundles stay thin, which is why the XX stack lives under `src/noise/` and never under `src/core/`.
- js-libp2p remains a documented fallback with an unchanged wire, and WebTransport remains a later framing option under transport-adapter ownership rather than a second default.
- The route decision can be revisited only on evidence of failed byte parity or unreliable framing; both were checked and passed, so the default stands.

Related: [the connect wire contract](../../../specs/connect/spoke-connect.md), [the Noise XX contract](../../../specs/connect/noise-xx-libp2p-contract.md), [the publish channel decision](../process/2026-08-02-connect-publish-channels.md), [the TS client SDK pattern](../../../knowledge/architecture-patterns/connect-ts-client-sdk.md).
