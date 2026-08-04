---
module: spoke-connect
date: 2026-08-04
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["porting the Noise XX transport to another language or runtime", "shipping an opt-in crypto subpath inside a published package", "proving wire interop against rust-libp2p without a live peer in CI"]
tags: [spoke-connect, noise-xx, libp2p, golden-transcript, interop, subpath, bundle-isolation, snow]
---

# Pure-TS Noise XX stack: rust-libp2p interop and opt-in subpath isolation

## Context

`@42ch/spoke-connect` ships a thin default client (WebSocket ordered-stream transport, JCS + Ed25519 identity, the session-core parity rules). Integrators that need to join a libp2p-noise mesh — the transport of the Rust reference `crates/spoke-connect`, which composes `libp2p::noise::Config::new` over libp2p 0.56.0 — get a first-party pure-TypeScript `Noise_XX_25519_ChaChaPoly_SHA256` stack behind the opt-in `./noise` subpath. Two hard problems were solved: (1) proving byte-level wire interop with rust-libp2p deterministically in CI without a live Rust peer; (2) shipping the crypto stack without widening the default bundle's dependency surface. Both solutions generalize to every future language Noise port and every future opt-in crypto subpath.

## Guidance

### 1. Golden-transcript interop via a snow-engine recorder

The interop gate replays a **recorded rust-libp2p Noise XX initiator transcript** against the TS responder. The recording is produced by a dev-only Rust example binary, `crates/spoke-connect/examples/noise_recorder.rs`:

- Recorder dependencies are **dev-dependencies only** (`snow 0.9.6` with `ring-resolver`, `libp2p-identity 0.2`, `x25519-dalek 2`), and `exclude = ["examples/**"]` keeps the binary out of the published crate tarball (`cargo package --list` verified). No Rust artifacts ship to consumers — only the committed JSON fixture under the TS test tree.
- The recorder drives `snow::HandshakeState` directly with the **exact engine behind libp2p-noise 0.46.1** (the crate behind the reference's `noise::Config::new`): same `snow` version and `ring-resolver` feature, same builder parameters libp2p-noise composes (`prologue([])`, `local_private_key(static_secret)`), and a `RecorderResolver` mirroring libp2p-noise's `protocol.rs::Resolver` (hash/cipher from `snow::resolvers::RingResolver`; X25519 DH over `x25519_dalek`).
- Ephemeral keys are pinned via snow's `fixed_ephemeral_key_for_testing_only` — the only pinning hook; libp2p-noise never exposes ephemerals, which is why the recorder drives `snow::HandshakeState` directly instead of the crate-private framed codec.
- Payload and signature replicate libp2p-noise `send_identity` exactly: `NoiseHandshakePayload { identity_key = libp2p-identity PublicKey protobuf, identity_sig = Ed25519 over "noise-libp2p-static-key:" || static_x25519_pub }`, no extensions, quick-protobuf field order.
- The recorder **self-verifies before emitting**: flight-1 payload empty, payloads cross-read intact, handshake hashes equal, remote statics crossed, both identity signatures verified exactly like libp2p-noise `finish()`, transport round-trip opens in both directions. Any mismatch panics — no fixture is written.
- Rerun is **byte-identical** (deterministic; verified by `diff`). The fixture (`tests/noise/fixtures/noise-xx-golden.json`) records **pure Noise frames after multistream** — u16-BE length-prefixed wire bytes with the `/noise` negotiation outside the Noise messages.

### 2. The interop assertions that prove parity

The TS responder runs against the recording and must satisfy all of:

- Flight-2 wire bytes reproduced **byte-for-byte** (the headline gate: state machine + payload protobuf + Ed25519 static-key signature all reproduce the rust-libp2p bytes exactly).
- Handshake hash identical; remote static public key and remote identity public key match the pinned values.
- **Split transport keys identical**: decrypting the recorded initiator→responder post-handshake frame yields the recorded plaintext, and re-sealing the recorded responder→initiator plaintext reproduces the recorded frame byte-for-byte.
- Remote `peer_id` matches the pinned initiator `peer_id`. Pinning the initiator identity seed to the golden-hello seed cross-validates identity derivation against the hello SSOT (`12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf`).

The interop test was written first (red on missing fixture) and passed green on first run against the genuine recording — the whole stack was wire-correct with no byte reconciliation.

### 3. u16-BE framing and identity payload with verify-before-use

**Framing** (handshake and transport share one codec): every Noise message is `| len_be: u16 | ciphertext |`. Max ciphertext 65535 (`MAX_NOISE_MSG_LEN`), extra encrypt headroom 1024, max cleartext per frame 65535 − 1024 = 64511 (`MAX_FRAME_LEN`). Cleartext larger than `MAX_FRAME_LEN` splits across frames; a reader with fewer than `2 + len` bytes buffered waits — no partial AEAD open. Post-handshake AEAD: key = split transport key for that direction, nonce = per-direction counter starting at 0, AAD empty, ciphertext = cleartext + 16 (Poly1305 tag).

**Identity payload**: Noise XX always carries a local static X25519 keypair in the `s` tokens (ephemeral-per-process, not the long-term Ed25519 key). Empty/`null` `s` is **not** wire-compatible with rust-libp2p. The `NoiseHandshakePayload` (flights 2 and 3; flight 1 is empty) binds the long-term Ed25519 identity to the Noise static key:

```
signed_preimage  = "noise-libp2p-static-key:" || static_public_x25519_bytes
identity_sig     = Ed25519.Sign(identity_private, signed_preimage)
```

**Verify-before-use** — after both payloads are received and XX completes: decode remote `identity_key` → Ed25519 public key → `PeerId`; verify `identity_sig` over the remote static public; on failure abort (authentication failure) and never open the transport. This is mandatory for mesh interop, not a shortcut the session layer can absorb: the reference calls `finish()`, which requires a verifiable `identity_key` + `identity_sig` and returns `BadSignature` / `AuthenticationFailed` otherwise. The Noise-authenticated `PeerId` is the transport identity the SPOKE hello gate compares against `peer_id` when an authenticated transport is present.

**Single-shot finish**: `NoiseHandshake.finish()` is memoized — the first invocation runs completion; every subsequent or concurrent call returns the same result, including the same cached rejection when the first attempt failed (a failed handshake is terminal). One shared `NoiseTransport` is returned, so a ChaCha nonce/key pair is never reused across completions.

### 4. Opt-in subpath isolation (three-layer gate)

Wiring: `package.json` `exports["./noise"]` mirrors the frozen condition shape (`import`/`require` × `types`/`default` → `dist/noise/index.{js,cjs,d.ts,d.cts}`); tsup gains a third entry `src/noise/index.ts`; `@noble/ciphers` + `@noble/curves` are declared package dependencies but imported only from `src/noise/**`; the noise graph never imports `src/core/**` (the session-core parity surface stays untouched).

Three complementary verification layers defend the published artifact, not just the source tree:

1. **Source-level dependency trace** (in CI unit suite): an esbuild metafile trace over the source entries with `packages: 'external'` (esbuild is a test-only devDependency; no build needed for CI). Bidirectional assertions — default `.` graph excludes `@noble/ciphers` / `@noble/curves` and contains no `src/noise/**` module; `./noise` graph includes both Noise-only deps and its own modules and excludes `src/core/**`. Positive controls on both sides (default graph still resolves `@noble/ed25519`; noise graph resolves its modules) keep the gate **non-vacuous**.
2. **Exports-map resolution smoke**: ESM + CJS `import`/`require` against the built `dist/` via Node self-reference. Root `.` exposes default symbols and no Noise symbols; `./noise` exposes the full public barrel while internal raw crypto (`encrypt`/`decrypt`/`dh`/`hkdf`) stays unexported.
3. **Dist-shape smoke** (`test:dist`, CI after build): root `.` resolves in ESM + CJS; `./noise` resolves in ESM + CJS with the complete barrel table.

The layer-3 gate caught a **real pre-existing published-shape defect**: `canonicalize@3.0.0` is ESM-only (`"type": "module"`, exports map with an `import` condition but no `require` condition), so the published CJS entry crashed any CJS consumer with `ERR_PACKAGE_PATH_NOT_EXPORTED` at module load. Fix: bundle the tiny dependency-free file into both formats (`noExternal: ["canonicalize"]` in `tsup.config.ts`). Source-level unit tests cannot see this class of defect — only a gate that resolves the built package shape can.

## Why This Matters

- **The interop gate is genuine, not synthetic**: the recorder drives the exact engine behind libp2p-noise with pinned keys and self-verifies before emitting; reruns are byte-identical. A fake-green fixture is structurally prevented.
- **Deterministic CI**: no live peer, no network, no randomness — the pure-TS stack is proven wire-compatible at the byte level before any product wiring.
- **Verify-before-use is load-bearing**: the transport identity binding (static key ↔ long-term Ed25519 ↔ `PeerId`) is what lets the SPOKE session layer trust the hello `peer_id` on an authenticated transport.
- **Subpath isolation preserves the thin-default contract**: the three-layer gate (source trace + exports resolution + dist shape) means a regression in either direction — Noise leaking into the default bundle or default deps creeping into Noise — fails CI.

## When to Apply

- Porting the Noise XX transport to another language or runtime (Kotlin, C#, Go, …): reuse the recorder + golden-transcript gate shape, starting from the frozen contract and the recorded fixture.
- Adding any opt-in crypto subpath to a published package: reuse the `exports` conditions + tsup multi-entry + trace + dist-shape gate stack.
- Changing the wire contract or bumping the engine line: re-record the fixture with the same recorder and `diff` it against the committed bytes before re-running the interop suite.
- Adding an optional dependency to the default graph: the isolation gate will fail.

## Examples

```ts
// Exports conditions — the isolation contract
// "exports": { "./noise": { "types": "./dist/noise/index.d.ts", "import": "./dist/noise/index.js", "require": "./dist/noise/index.cjs", "default": "./dist/noise/index.js" } }

import { Session } from '@42ch/spoke-connect'                // thin default
import { NoiseXX, NoiseTransport } from '@42ch/spoke-connect/noise'  // opt-in mesh
```

```text
=== DEFAULT . graph (16 src files) ===
external deps: @noble/ed25519, @noble/hashes/sha2.js, canonicalize
noble/ciphers present: false   noble/curves present: false

=== NOISE graph (7 src files) ===
src files: src/crypto.ts, src/identity.ts, src/noise/{crypto,framing,index,payload,xx}.ts
external deps: @noble/ciphers/chacha.js, @noble/curves/ed25519.js, @noble/ed25519,
               @noble/hashes/{hkdf,sha2,utils}.js
noble/ciphers present: true    noble/curves present: true    core/ present: false
```

```text
| len_be: u16 | ciphertext: len bytes |     // handshake and transport frames
// max ciphertext 65535; max cleartext per frame 64511 (65535 − 1024 headroom)
```

## See also

- [`noise-xx-libp2p-contract.md`](../../specs/noise-xx-libp2p-contract.md) — the frozen wire contract (XX pattern, framing constants, payload protobuf, domain-separated signature, verification gates).
- [`connect-ts-client-sdk.md`](connect-ts-client-sdk.md) — the thin default surface the subpath must never widen.
- [`connect-golden-vector-ssot.md`](connect-golden-vector-ssot.md) — golden-fixture SSOT discipline for cross-language constants (the hello SSOT the recorder's pinned identity cross-validates against).
- [`spoke-connect-libp2p-spike.md`](spoke-connect-libp2p-spike.md) — the Rust-side transport the TS stack interops with (allowlist timing, identify↔noise binding).
