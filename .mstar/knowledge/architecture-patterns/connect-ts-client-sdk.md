---
module: spoke-connect
date: 2026-08-01
last_updated: 2026-08-03
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["building the first language-SDK slice for spoke-connect", "porting the connect session core to a new language", "choosing crypto / JCS / framing dependencies for a connect client", "adding a browser-or-Node connect client", "extending session-core capability parity across TS and Rust"]
tags: [spoke-connect, connect-ts, path-a, pure-ts, websocket-framing, jcs, webcrypto, golden-vectors, session-core-parity]
---

# pure-TS-minimal connect client SDK (first slice)

## Context

The TypeScript connectivity route (`.mstar/specs/spoke-connect-ts-route.md`) locks **pure-TS-minimal** as the primary Path A client stack: WebSocket as an ordered reliable stream carrying one JSON connect envelope per message, WebCrypto Ed25519 with an `@noble/ed25519` fallback, RFC 8785 JCS canonicalization, and a small behavior port of the Rust session core — no js-libp2p mesh, no WASM. Identity-byte reproducibility in JavaScript was proven first by the throwaway proof (`tooling/connect-identity-proof/proof.mjs`), and `packages/spoke-connect-ts` (`@42ch/spoke-connect-ts`) is the landed first slice: a workspace-private client library whose `src/core/` mirrors the pure Rust core accept/reject outcomes without sharing code.

## Guidance

### Package layout (workspace-private, lockstep)

| Decision | Rule |
|---|---|
| Visibility | `"private": true` — ships inside the monorepo; not published (future publish is governed by the connect publish strategy) |
| Version | Tracks the monorepo lockstep SemVer — asserted by `verify:version`, bumped by `release:bump` |
| Engine floor | `node >= 20.19.0` (see Node floor rationale below) |
| Module style | ESM (`"type": "module"`); monorepo-internal imports resolve TypeScript sources under `NodeNext` |
| Isomorphism | `src/` modules (`identity`, `crypto`, `jcs`, `framing`, `core/`) are browser-swappable — no `ws` import; Node-only pieces (`ws` adapter, `connectClient`) live in the `src/node/` subpath, not the root barrel |
| Root barrel | Exports identity, crypto, framing, JCS, and the whole core barrel; `src/node/` is imported by explicit subpath |

### Identity port provenance

`derivePeerIdFromEd25519Pubkey` (protobuf `PublicKey` → identity multihash `0x00` → base58btc, no multibase prefix) is ported from `tooling/connect-identity-proof/proof.mjs`; the normative formula lives in the spec §Identity binding. Golden constants are **redeclared once in `src/golden.ts` with provenance comments** pointing at the Rust core test constants (`peer_id.rs`, `hello_crypto.rs`) — tests never recompute values from the code under test.

### Crypto matrix

- **WebCrypto Ed25519 is primary** (`subtle.sign` / `subtle.verify`) on the same code path as the fallback; raw keys cross as DER-wrapped (`ed25519SeedToPkcs8` / `ed25519PubkeyToSpki` hand-encode the RFC 8410 headers — no PEM library).
- **`@noble/ed25519` fallback** for runtimes without WebCrypto Ed25519; forced-`@noble` helpers stay exported for tests and fallback verification but are not client surface.
- **base64url without padding** — padding changes the wire `signature`.

### JCS pin + golden-byte gate

`canonicalHelloBytes(peerId, nonce, host)` canonicalizes **exactly** `{protocol_version, peer_id, nonce, host}` (top-level `extensions` and `signature` excluded) with the pinned `canonicalize@3.0.0` package. Tests assert the output against `GOLDEN_JCS_HEX` (264 bytes) — the **golden-byte gate**: if a dependency update ever diverges, port the hand-rolled JCS subset from the proof script into the module instead of weakening the golden vector. Absent optional manifest members (e.g. `host.authority`) are omitted, never emitted as `null` — `canonicalize` drops `undefined` members, matching Rust `skip_serializing_if`; an explicit `null` would change the canonical bytes and break verify against Rust.

### WS framing (one JSON document per message, fail-closed)

`encodeJsonMessage` / `decodeJsonMessage` implement the spec §Transport framing unit: one text frame per envelope (binary frames decoded as UTF-8). **No batching** — `JSON.parse` consumes the entire payload, so a trailing second document in one frame throws; `JSON.stringify` returning `undefined` throws at encode. Envelope types are discriminated by JSON shape (guard functions in the client), not by a framing header.

### Session-core TS port surface

The `src/core/` barrel mirrors the Rust core's `pub use` block function-for-function; it is a **behavior port** (same accept/reject outcomes), not shared code:

| Rust core (`crates/spoke-connect/src/core/`) | TS (`src/core/`) |
|---|---|
| `PROTOCOL_VERSION` | `PROTOCOL_VERSION` |
| `CoreError` / `CoreInvokeError` | `CoreError` / `CoreInvokeError` (+ code types) |
| `OutboundSequence` / `InboundSequence` / `MAX_SEQUENCE` | same names — start 0, exhaustion instead of wrap past 2⁵³−1 |
| `check_response_correlation` (+ `Correlation`) | `checkResponseCorrelation` (+ `correlationFromRequest` / `correlationFromResponse`) |
| `dispatch_allowed` / `required_capability` / `CAPABILITY_*` | `dispatchAllowed` / `requiredCapability` / `CAPABILITY_*` — unknown op fails closed |
| `NonceStore` (+ `generate_nonce`) | `NonceStore` (+ `generateNonce`) — per-sender `(peer_id, nonce)` |
| `is_allowlisted` | `isAllowlisted` — empty allowlist rejects all |
| `sign_hello_ed25519` / `verify_hello_ed25519` | `signHelloEd25519` / `verifyHelloEd25519` |
| `issue_capability_token` / `verify_capability_token` (+ `CapabilityClaims` / `CapabilityTokenProof` / `TOKEN_VERSION` / `CLOCK_SKEW_SECONDS`) | `issueCapabilityToken` / `verifyCapabilityToken` (+ same types / constants) — async on TS (WebCrypto / `@noble`), same bytes; fail-closed proof-shape guard before crypto |
| `token_authorizes_op` | `tokenAuthorizesOp` — membership / subset-of-grant against token grants |
| `ed25519_pubkey_from_peer_id` | `ed25519PubkeyFromPeerId` — reverse of derive; required by capability-token verify (issuer key recovery from `claims.iss`) |
| (session helper) | `Session` / `negotiatedCapabilities` |

### Session-core parity discipline

Normative sources (do not duplicate wire contracts here): root `AGENTS.md`
(connect boundary bullet) and
[`.mstar/specs/spoke-connect-ts-route.md`](../../specs/spoke-connect-ts-route.md)
§Session-core parity. The discipline is how Path A keeps **capability parity**
with the Rust reference without sharing code.

**1. Enumerate the parity surface (in vs out)**

| In (same accept/reject rules on both sides) | Out (asymmetric by design) |
|---------------------------------------------|----------------------------|
| allowlist | Transport: WebSocket (TS) vs libp2p stack (Rust) |
| `peer_id` — derive **and** reverse (token verify) | Node lifecycle / dial / listen |
| hello crypto (sign/verify over JCS) | uniffi FFI facade (Path B only) |
| nonce single-use store | Package-layout helpers unique to one language (`Session` thin helper, correlation builders, `generateNonce`) |
| request correlation | |
| outbound / inbound sequence | |
| capability-token auth (issue/verify, claims/proof shape) | |
| dispatch gate + product-op capability map, including `tokenAuthorizesOp` membership against token grants | |

Re-diff the Rust `crates/spoke-connect/src/core/` barrel against the TS
`src/core/` (+ `src/identity.ts` for `peer_id`) whenever either side gains a
session rule. Status each item: **parity** (matching semantics present) or
**asymmetric-by-design** (deliberate shape difference, not a gap). Gaps on
the pure-rules list are parity work; transport differences are not.

**2. Port pure session logic only**

- Behavior port: same accept/reject outcomes, same fail-closed defaults
  (empty allowlist rejects all; unknown op fails closed; deny-unknown-fields
  on token claims/proof).
- Keep crypto/JCS/framing on the existing TS helpers; do not reimplement
  platform crypto inside the core module under test.
- Async on TS (WebCrypto / `@noble`) vs sync on Rust is fine when the **bytes**
  and outcomes match.
- Product-configured op→capability maps stay host-owned on both sides; the
  core table covers baseline ops only.

**3. Prove parity (two layers)**

| Layer | What it pins | Pattern |
|-------|--------------|---------|
| Same-language round-trip | Issue/verify, reject paths, `tokenAuthorizesOp` membership cases | Unit tests on each side mirroring the other's cases (deterministic seeds, fixed `now`) |
| Cross-language golden fixture | JCS canonical bytes + signature encoding the other language cannot see from its own round-trip | One side **mints**, the other **verifies** a checked-in proof |

Capability-token golden pattern (landed): Rust mints a proof from the
identity golden seed (`tooling/connect-identity-proof/` bytes `1..=32` →
issuer `GOLDEN_PEER_ID`) with deterministic subject/audience role seeds;
the proof JSON is checked in as
`packages/spoke-connect-ts/tests/fixtures/capability-token-golden.json`;
TS `verifyCapabilityToken` must accept it and return the granted
capabilities. Identity/hello goldens stay in `src/golden.ts` with provenance
comments pointing at Rust core constants — tests never recompute the pinned
bytes from the code under test.

When adding a new pure-core rule: extend the surface table, port the
function, add mirror unit cases, and — if the rule emits or consumes
cross-language bytes (JCS, sig, peer_id) — add or extend a Rust-produced
(or TS-produced) golden fixture the other side verifies.

### connectClient flow

`connectClient({ url, identity, manifest, remotePubkey, allowlist })`:

1. **Dial** with a bounded wait (default 5 s); on failure the socket is closed so the peer sees a clean disconnect.
2. **Signed hello**: derive the local `peer_id`, require the remote `peer_id` on the allowlist *before* dialing anything (fail-closed), send the signed hello.
3. **Server verify**: the server's hello is verified against the preconfigured `remotePubkey` (how the key is obtained is transport-adapter-owned) and its `peer_id` must be allowlisted; the server's manifest is taken only from its **signed** hello.
4. **Session snapshot**: accept `ConnectSession` only when `initiator_peer_id` / `responder_peer_id` equal the authenticated hello peer ids, `session_id` is non-empty, and `initial_sequence` is 0 (protocol v1 `const 0`).
5. **Invoke**: allocate the next outbound sequence (first = 0), attach a fresh `request_id`, correlate the response's echo fields. **Promise-only API** — sequence exhaustion and sync send failures reject, never throw synchronously. Pending invokes key by `request_id` with per-invoke bounded waits; socket error/close fails all pending immediately.
6. **Handshake ordering**: a waiter queue consumes hello/snapshot frames in order — several frames arriving in one TCP segment are buffered until their waiter registers; after the session is established, stray non-invoke envelopes are dropped (no buffering, no unbounded inbox). A malformed JSON frame fails all pending and closes the socket.

### Two-node in-process WS test topology

`tests/two-node.test.ts` runs an in-process `ws` server + client over `127.0.0.1:<ephemeral>` — **bounded waits only, no sleeps**: every await races a timeout deadline. A minimal handshake-server fixture answers the client's signed hello with its own signed hello + snapshot (server-side hello gates deliberately skipped — those are covered by the interop test), and a separate robustness suite drives dial failure, malformed frames, handshake rejection, and close semantics.

### Golden-vector discipline: two manifests, one byte contract

- `goldenManifest()` — the **byte-pinned contract**: its canonical bytes are `GOLDEN_JCS_HEX` and must never change. It carries an empty `namespaces` list, which diverges from the generated type's `minItems 1` (cast at the type level) — the JCS bytes are the pinned contract.
- `schemaConformantManifest()` — the **schema-valid fixture** for tests that put a hello on the wire or claim wire/schema conformance (`namespaces: ["toy_world"]`, valid roles/capabilities, no casts).

Keep the byte contract and the schema-valid fixture separate; they serve different test classes and conflating them silently blesses a schema drift.

### Node floor ≥ 20.19 rationale

20.19.0 is the `@noble/hashes` floor **and** the first Node line that accepts WebCrypto Ed25519. CI runs the suite on Node 20.x — WebCrypto Ed25519 on ≥ 20.19, `@noble/ed25519` on older patches; the fallback keeps the same seed/sign semantics.

## Why This Matters

Envelope-level interop with Rust peers is the v1 goal: same hello bytes, same `peer_id` derivation, same session-core accept/reject outcomes — over any ordered reliable stream. The slice proves Path A works without a Rust runtime and with a deliberately thin dependency tree (platform crypto + `@noble` + pinned JCS + `ws`), which keeps a future thin `@42ch/*` helper embeddable. The decisions here — package shape, crypto matrix, JCS pin with a byte gate, fail-closed framing, the port-surface table, the two-fixture golden discipline — are the checklist any other language SDK slice repeats.

## When to Apply

- Building any future language SDK slice (another Path A port, or uniffi targets beyond Swift) — reuse the slice shape: provenance-pinned golden constants, crypto primary+fallback matrix, canonicalization pin with a byte gate, one-document-per-message framing, a core surface that mirrors the Rust barrel, bounded-wait test topology, and the byte-contract vs schema-valid fixture split.
- Extending or auditing TS↔Rust session-core capability parity — enumerate the pure-rules surface, port behavior (not transport), prove with mirror unit tests plus a cross-language minted golden fixture.
- Choosing dependencies for a connect client — prefer platform crypto and a pinned canonicalizer over a swarm stack; keep the JCS implementation behind a golden-byte gate.
- Adding a browser or Node connect client — the isomorphic `src/` modules are the swappable core; transport adapters (native WebSocket vs `ws`) plug in at the `src/node/` boundary.

## Examples

### The two-fixture split (excerpt from `src/golden.ts`)

```ts
goldenManifest(): HostCapabilityManifest // byte contract — namespaces: [] cast past minItems 1;
                                         // GOLDEN_JCS_HEX pins its historical Rust bytes
schemaConformantManifest(): HostCapabilityManifest // schema-valid: namespaces: ["toy_world"], no casts
```

### Handshake-phase inbox (frame ordering in one TCP segment)

The receiver buffers hello/snapshot frames that arrive ahead of their waiter (several frames emitted in the same macrotask); after `sessionEstablished`, non-invoke envelopes are dropped instead of buffered — the inbox cannot grow without bound post-handshake.

## See also

- [`connect-identity-parity-proof.md`](../testing-patterns/connect-identity-parity-proof.md) — the proof script and port gotchas this slice implements (protobuf PublicKey, identity multihash, base64url, DER wrap).
- [`connect-session-core-ffi-boundary.md`](connect-session-core-ffi-boundary.md) — the Rust core whose surface this package mirrors (Path A vs Path B parity).
- [`connect-capability-token-auth.md`](connect-capability-token-auth.md) — capability-token claims/proof design and validation order the parity surface includes.
- [`spoke-connect-wire-and-auth.md`](spoke-connect-wire-and-auth.md) — the wire family, JCS signed-field set, and identity binding the client implements.
- [`spoke-connect-ts-route.md`](../../specs/spoke-connect-ts-route.md) — the route decision and session-core parity rule this package implements (pure-TS-minimal; fallback js-libp2p).
