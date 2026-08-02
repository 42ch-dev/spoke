# SPOKE Connect — TypeScript connectivity route

**Status:** Informative decision — does not change connect envelopes.

**Normative wire / identity / framing:** [spoke-connect.md](spoke-connect.md) §Identity binding, §Signature canonicalization, §Transport framing, §Embedding model. This document chooses a **client stack route** for browser and Node integrators; it does not fork identity rules or add schemas.

**Updated:** 2026-08-01

---

## Locked recommendation

| Item | Value |
|------|-------|
| **Primary route** | **pure-TS-minimal** |
| **Transport (first slice)** | WebSocket as an ordered reliable stream carrying one JSON connect envelope per message (P0 §Transport framing) |
| **Crypto** | WebCrypto `Ed25519` where available; fallback `@noble/ed25519` (same seed/sign semantics) |
| **Canonicalization** | RFC 8785 JCS over `{protocol_version, peer_id, nonce, host}` — pure implementation or a maintained JCS library proven against golden vectors |
| **`peer_id`** | Spec formula only: protobuf `PublicKey` → identity multihash `0x00` → base58btc (no multibase prefix, no CIDv1) |
| **Fallback route** | **js-libp2p** when the product goal is direct Noise/yamux mesh interop with the rust-libp2p reference stack on a shared libp2p network (not only envelope-level interop over WebSocket) |
| **Deferred** | **WASM** (Rust session core compiled to WASM + JS transport) — only if a future pure-TS crypto/JCS gap appears that WebCrypto/`@noble` cannot close; toolchain weight is high for this protocol repository |

Identity-byte reproducibility in JavaScript is **proven** (see [Identity proof evidence](#identity-proof-evidence)). No overturn of the pure-TS-minimal default.

---

## Evaluation criteria (C1)

Scoring is High / Med / Low fitness for SPOKE Path A browser+Node clients that must match Rust hello/`peer_id` bytes. Not a summed numeric total.

| # | Criterion | js-libp2p | WASM (Rust core + JS transport) | pure-TS-minimal |
|---|-----------|-----------|----------------------------------|-----------------|
| 1 | **Transport — browser** | **High** — WebSocket / WebRTC transports; ordered streams via muxers | **Med** — still needs a JS transport (WS/WebRTC); WASM is not a transport | **High** — WebSocket (and later WebTransport) as ordered reliable framing; matches P0 stream model without a mesh |
| 2 | **Transport — Node** | **High** — TCP + WS; operational surface of a full swarm | **Med** — same JS adapter story; WASM load simpler on Node than browser | **High** — WS first; TCP optional later if a host needs it; low operational surface |
| 3 | **Identity / PeerId** | **High** — `@libp2p/peer-id` / multiformats can match libp2p PeerId strings; still must assert against SPOKE golden vectors | **High** — reuses Rust `derive_peer_id_from_ed25519_pubkey` | **High** — pure derivation matches golden `12D3KooWJ1T…` (proof) |
| 4 | **Signing** | **Med** — stack has keys/sign primitives, but hello is SPOKE JCS+base64url, not a libp2p native hello; custom sign path still required | **High** — Rust `sign_hello_ed25519` / verify | **High** — WebCrypto Ed25519 (Node 24+, modern browsers) + noble fallback; 64 raw bytes → base64url no pad (proof) |
| 5 | **JCS (RFC 8785)** | **Med** — external JCS dependency still required; must match `serde_jcs` | **High** — `serde_jcs` in-core | **High** — pure JCS subset (or lib) matches golden hex vector (proof); omit absent optionals (no `"authority": null`) |
| 6 | **Session-core port cost** | **Med** — session rules still ported in TS; libp2p does not replace sequence/nonce/allowlist/correlation | **High** — reuse pure Rust core behind WASM | **Med** — small pure port (sequence, nonce, allowlist, correlation, dispatch gate); intentional and bounded |
| 7 | **Dependency weight** | **Low** — deep `@libp2p/*` tree, large supply-chain and bundle surface for a future thin `@42ch/*` helper | **Low** — WASM binary + wasm-bindgen glue + JS transport | **High** — WebCrypto (platform) + optional noble + thin WS; no swarm stack |
| 8 | **Ecosystem / maintenance** | **Med** — js-libp2p remains active (e.g. 3.x releases); Noise/multistream interop with rust-libp2p 0.56 needs version pinning and periodic re-verify | **Low** — wasm-pack/wasm-bindgen CI and artifact policy inside a protocol repo is ongoing cost | **High** — platform crypto + small pure modules; low churn surface |
| 9 | **Fit with P0 embedding model** | **Med** — valid Path A host, but nudges integrators toward “connect = libp2p mesh” rather than envelope-on-stream | **Low** — Path B-ish reuse of Rust; browsers that refuse WASM still need a pure path | **High** — Path A purity: no Rust required; no daemon; transport-adapter-owned framing |
| 10 | **Interop with Rust reference spike** | **Med–High** — Noise mesh peer possible when versions align; envelope interop still needs SPOKE hello bytes | **High** — same core bytes as the spike | **High** for **envelope-level** interop (same hello/`peer_id`/session rules over any ordered stream). Noise multistream is **not** required for v1 product goals that only need connect envelopes |

### Synthesis per route

**js-libp2p.** Strong when the product must join a shared libp2p network and speak Noise/yamux with the Rust reference stack. Weak as the *default* first TS slice: heavy deps, and SPOKE still owns JCS hello + session-core outside the mesh. Use as the **mesh fallback**, not the default Path A client.

**WASM.** Best byte-parity and session-core reuse, but does not remove the need for a JS transport adapter, adds toolchain/binary weight to a protocol repository that does not ship a connect runtime package, and fails browsers or hosts that decline WASM. Defer unless pure-TS crypto or JCS cannot match Rust (T2 shows they can).

**pure-TS-minimal.** Best default for Path A browser/Node integrators: WebSocket ordered framing, WebCrypto/`@noble` Ed25519, JCS, and a small session-core port. Identity proof confirms the P0 formula is implementable without Rust. Envelope-level interop with Rust peers is the v1 goal; full Noise mesh remains an optional later path via js-libp2p.

---

## Rationale (recommendation)

1. **P0 contract is language-neutral.** Connect envelopes ride an ordered reliable stream; libp2p is one reference binding, not the wire definition ([spoke-connect.md](spoke-connect.md) §Embedding model, §Transport framing).
2. **Identity is reproducible in JS.** The throwaway proof matches Rust golden vectors for `peer_id`, JCS bytes, Ed25519 signature, and base64url encoding — so pure-TS is not blocked on crypto or PeerId math.
3. **Lowest coupling and dependency weight** for a future private TS helper that products may embed; matches the repo pattern of thin `@42ch/*` packages without introducing a swarm runtime into the protocol tree.
4. **Fallback is explicit.** Products that need Noise mesh interop with `crates/spoke-connect` choose js-libp2p without changing wire schemas. WASM stays a contingency, not the first cut.

**Overturn check:** C2 allowed overturning pure-TS-minimal only if identity-byte parity or framing reliability failed. T2 **passed**; no overturn.

---

## Fallback paths

| Condition | Route |
|-----------|--------|
| Product requires dial/listen on the same libp2p mesh as the Rust spike (Noise + yamux + identify) | Adopt **js-libp2p** for transport; keep SPOKE hello JCS + `peer_id` rules unchanged |
| A target runtime lacks WebCrypto Ed25519 and `@noble/ed25519` is unacceptable | Evaluate **WASM** crypto or full core; document matrix in the TS SDK plan |
| Pure-TS JCS diverges on a new host field shape | Fix the TS JCS path or golden vectors first; do not fork signed-field sets |

---

## Identity proof evidence

**Location:** [`tooling/connect-identity-proof/`](../../tooling/connect-identity-proof/) (not a workspace package; not published; not CI-gated).

**Command:**

```bash
node tooling/connect-identity-proof/proof.mjs
```

**Runtime:** Node v24.18.0 (WebCrypto `Ed25519` available). Zero npm dependencies (PKCS8/SPKI wrap of raw seed/pubkey + pure JCS + pure base58btc).

**Golden inputs** (aligned with `crates/spoke-connect` core tests):

| Item | Value |
|------|--------|
| Seed | bytes `01`…`20` (1..=32) |
| Public key | `79b5562e8fe654f94078b112e8a98ba7901f853ae695bed7e0e3910bad049664` |
| `peer_id` | `12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf` |
| Nonce | `golden-nonce-000000000001` |
| Host | `host_id=golden-host`, `capabilities=["spoke-baseline"]`, `roles=["data-store"]`, empty `namespaces`/`extensions`, `schema_version=1`, **authority omitted** |
| Signature (base64url no pad) | `yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg` |

**Result (all PASS):**

```text
[PASS] peer_id derivation — 12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf
[PASS] JCS UTF-8 bytes — 264 bytes match golden hex
[PASS] Ed25519 sign (64 raw bytes) — 64 bytes
[PASS] base64url (no pad) signature — yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg
[PASS] Ed25519 verify golden signature — verified
[PASS] local JCS equals golden hex decode
RESULT: ALL CHECKS PASSED
```

**Porter note:** Porters should omit absent optional host fields (e.g. `authority`) from the signed object rather than serializing them as JSON `null`; emitting `null` changes the canonical JCS bytes and breaks signature verify against Rust.

---

## TypeScript connect first-slice scope

**Shipped slice:** `packages/spoke-connect-ts` — a workspace-private, product-facing TS helper, not a published connect daemon. Capability-token remains the pending parity item (see item 1 and item 6); the rest of items 1–5 is implemented.

1. **Package shape (suggested):** private workspace or consumer-owned module — pure helpers first (`derivePeerId`, `canonicalHelloBytes` / JCS, `signHello` / `verifyHello`, base64url). No swarm, no mDNS/DHT. `capability-token` is normative; the TS SDK slice does not implement it yet.
2. **Transport adapter:** WebSocket client that sends/receives one JSON document per message; map to hello → session establish → invoke request/response correlation via existing wire fields only.
3. **Session-core port (minimal):** protocol version check, allowlist on `peer_id`, nonce single-use, per-direction sequence, `request_id` correlation, dispatch gate — mirror [spoke-connect.md](spoke-connect.md) session-core rules; keep I/O at the adapter boundary.
4. **Crypto matrix:** WebCrypto Ed25519 primary; `@noble/ed25519` fallback for older runtimes; keep the identity proof (or promote its vectors) as a regression check.
5. **Interop target:** envelope-level parity with Rust peers over WS (or a test double stream). Noise/js-libp2p mesh is an optional second track, not a blocker for slice 1.
6. **Out of first slice:** published npm connect package, CI-enforced identity gate (optional later), DHT discovery, capability-token auth (normative; not yet implemented by the TS SDK slice), full Noise stack in pure TS.

---

## Links

| Resource | Role |
|----------|------|
| [spoke-connect.md](spoke-connect.md) §Identity binding | Normative `peer_id` + Ed25519 |
| [spoke-connect.md](spoke-connect.md) §Signature canonicalization | `spoke-connect-hello-jcs-v1` signed field set + JCS + base64url |
| [spoke-connect.md](spoke-connect.md) §Transport framing | One JSON document per message; ordered reliable stream |
| [spoke-connect.md](spoke-connect.md) §Embedding model | Path A / Path B purity rules |
| `crates/spoke-connect` core (`peer_id`, `hello_crypto`) | Rust golden vectors + pure session core reference |
| `tooling/connect-identity-proof/` | Local JS reproducibility proof |
