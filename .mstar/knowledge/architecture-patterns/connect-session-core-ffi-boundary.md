---
module: spoke-connect
date: 2026-08-01
last_updated: 2026-08-02
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["extracting a pure session core for cross-language binding", "preparing a Rust crate for a uniffi FFI boundary", "deciding sync vs async surfaces on a foreign-language facade", "landing a first uniffi binding skeleton (Swift, Kotlin, …)", "verifying a community bindgen pipeline against the repo's pinned uniffi version"]
tags: [spoke-connect, session-core, ffi-boundary, uniffi, swift, path-b, golden-vectors, csharp]
---

# connect session-core extraction and FFI facade boundary

## Context

The connect embedding model (`.mstar/specs/spoke-connect.md` §Embedding model) offers **Path B — shared core bindings**: export a session-core implementation (Rust via uniffi) into a foreign host language, keeping the transport adapter in the host language or in the shared core. Before any binding can ship, the session rules must exist as a **pure, transport-free module** — otherwise every language target drags in a tokio/libp2p runtime. The `crates/spoke-connect` reference stack therefore extracted `src/core/` as the language-portable session rules; the same module doubles as a behavioral reference for **Path A** ports (language-direct implementations) that must match its accept/reject outcomes without sharing code. The first binding surface is deliberately **synchronous**: bridging a tokio runtime across an FFI boundary is the highest-risk part of the design, so the async node surface is deferred to a later slice instead of blocking the sync core.

A **Swift sync-core skeleton has landed** on that boundary: the session rules export through uniffi behind the optional `ffi` feature as a `cdylib`, Swift bindings generate from that library, and a macOS-local smoke asserts golden-vector parity from the Swift side. The binding is **core-only** — async node lifecycle stays Rust-side.

## Guidance

### Module boundary: pure core, thin transport adapters

Keep the session core free of transport and runtime concerns; make everything it needs to cross a language boundary plain and byte-oriented:

| In the core (pure, sync) | Out of the core (transport-owned) |
|---|---|
| `peer_id` derivation (`derive_peer_id_from_ed25519_pubkey`) | Node start / listen / shutdown (tokio) |
| Hello JCS sign/verify over **raw** 32-byte Ed25519 keys | Dialing, session establishment, invoke round-trips |
| Nonce single-use store, allowlist check | `libp2p::PeerId` ↔ `String` conversion |
| Outbound sequence allocation / inbound sequence advance | Noise / yamux / identify / request-response behaviours |
| Response correlation (`session_id` / `sequence` / `request_id` echo) | Delimiter, framing, payload-size backstops |
| Op dispatch gate (`dispatch_allowed`, `required_capability`) | Any I/O |

The core operates only on `spoke-schemas` connect types, `String` peer ids, `&[u8; 32]` keys, and `serde_json::Value` payloads — no `Multiaddr`, swarm, or libp2p types in its public surface. That property is what makes it a first-binding surface: a binding skeleton can export it as-is.

### Golden-vectors-first (capture before you delete)

When extracting a pure module from a working implementation, capture golden vectors from the **existing** path first, commit them as constants, and only then delete the old code. The connect core captured, before the transport cutover: libp2p `PeerId::to_string()` for a fixed Ed25519 key, `Keypair::sign` over `serde_jcs` bytes, and the canonical JCS hex itself (seed bytes 1..=32 → pubkey `79b5562e…` → peer id `12D3KooWJ1T…`; 264-byte JCS hex; base64url signature `yWu5Dl0jcK…`). Tests assert against those committed constants — never against values recomputed by the code under test, which would silently bless a regression.

### Sync vs async FFI split

Decide the boundary once and record it where bindings will be built (the crate README "Binding facade" section):

| Surface | Sync or async | FFI rule |
|---|---|---|
| Session rules (derive, sign/verify, nonce, allowlist, sequence, correlation, dispatch gate) | **Sync** | First binding surface; exported as sync FFI functions / objects; safe on any foreign caller thread; no tokio involved |
| Node lifecycle (start / listen / shutdown), `connect`, invoke wait | **Async** (tokio) | Never block foreign UI threads on the swarm loop; the landed binding is **core-only** (host-language transport + Rust core); a uniffi async/foreign-runtime bridge for a thin node is a deferred option |
| `invoke_handler` hook | Sync **on the event loop** today | Product handlers must return promptly and must not block on I/O; before exposing handlers over FFI, dispatch moves off the loop (`spawn_blocking`) |

Keys cross the boundary as raw bytes (`&[u8; 32]`), peer ids as strings, payloads as `serde_json::Value` — the first skeleton needs no `Multiaddr` / swarm types on FFI.

### Landed Swift skeleton: uniffi proc-macro, JSON-string boundary

The first binding (Swift, macOS) ships behind the non-default `ffi` feature via **uniffi 0.32 proc-macros — no `.udl` file**: `#[uniffi::export]` on free functions and object impl blocks, `#[derive(uniffi::Object)]` + `#[uniffi::constructor]` for stateful objects, `#[derive(uniffi::Error)]` for the error enums. The `bindgen-cli` feature (`uniffi/cli`) builds a crate-local `uniffi-bindgen` binary; generation runs `generate --library <cdylib>` against the `ffi`-built `cdylib` (`crate-type = ["rlib", "cdylib"]`), so the uniffi 0.32 line needs no installable CLI crate.

Boundary conventions the wrapper encodes:

- **Keys** cross as `Vec<u8>` (uniffi has no fixed-size array type) and are validated to exactly 32 bytes inside the wrapper (`CoreError::Crypto` on wrong length).
- **Peer ids** cross as `String`; the **host manifest and the hello envelope cross as JSON strings**, deserialized with `serde_json` inside Rust — no generated schema types appear on the FFI surface (`Multiaddr`, swarm, libp2p, and `spoke-schemas` types stay Rust-side). A JSON parse failure maps to `HandshakeFailed`.
- **Correlation** is flattened to primitives (`checkResponseCorrelation(expected_session_id, expected_sequence, expected_request_id, actual_…)`).
- **Error enums map variant-for-variant** (landed): `CoreError` → Swift `CoreError` (`InvalidHelloSignature`, `NonceReplay`, `HandshakeFailed(reason:)`, `InvalidNonce(message:)`, `Crypto(message:)`, `Jcs(message:)`, `TokenInvalid(message:)`); `CoreInvokeError` → Swift `CoreInvokeError` (`SequenceExhausted`, `InboundSequenceMismatch(expected:actual:)`, `CorrelationMismatch`).
- **Generated Swift bindings are gitignored** (`bindings/swift/generated/`) and regenerated by the documented script step (`uniffi-bindgen generate --library …`); the dylib install name is rewritten to `@rpath` so the smoke is not pinned to one machine.
- **macOS smoke** (`bindings/swift/Smoke/main.swift`) derives the golden peer id from the golden Ed25519 seed, signs/verifies the golden hello (asserting base64url signature parity with the Rust core), and exercises the rest of the exported surface — allowlist, sequences, nonce store, dispatch gate, correlation, protocol version — with the mapped error cases; every check prints `PASS`.
- **CI is ubuntu-only for the Rust export surface** (`cargo build` + `cargo test -p spoke-connect --features ffi`); the Swift toolchain and the smoke stay macOS-local — a locked decision, not an omission.

### Target-language matrix

The matrix records the landing order (priority per product direction, 2026-08-02: C#, Go, Python, Swift, Kotlin). All five targets are landed; each package ships on its own channel under the lockstep tag gate — see [`connect-publish-strategy.md`](../../specs/connect-publish-strategy.md) §7 for the current channel matrix.

| Language | Embedding path | Priority |
|---|---|---|
| C# | Path B uniffi | **First — landed** — desktop/server hosts; generated binding + net8.0 golden-parity smoke via a vendored `uniffi-bindgen-cs` fork retargeted to uniffi 0.32 (see binding-pipeline verification below); NuGet `42ch.Spoke.Connect` on GitHub Packages |
| Go | Path B uniffi | Second — **landed** — Go modules under `crates/spoke-connect/bindings/go` (golden-parity smoke) |
| Python | Path B uniffi | Third — **landed** — PyPI `spoke-connect` platform wheels via Trusted Publishing OIDC (golden-parity smoke) |
| Swift (iOS / macOS) | Path B uniffi | Fourth — **landed** — SPM product `SpokeConnect` from the repo at `vX.Y.Z` tags |
| Kotlin (Android) | Path B uniffi | Fifth — **landed** — GitHub Packages Maven `dev.42ch:spoke-connect` via `publish-maven` |
| TypeScript (browser / Node) | **Path A** language-direct | Parallel track — no uniffi/WASM assumed; decided by the TS route |

### Binding pipeline verification (community bindgens lag uniffi)

Community uniffi bindgen tools (C#/Go/Python) may lag the repo's pinned uniffi line — uniffi metadata encoding and runtime contract checksums change between uniffi versions, so a bindgen built for an older uniffi fails against a well-formed current `cdylib`. **Verify the full pipeline — generate → compile → link/load → runtime checksum — against the pinned uniffi version before planning a binding slice**; do not assume a published bindgen tag tracks the newest uniffi. Use a positive control (the crate-local uniffi-bindgen generating another language from the same `cdylib`) to confirm the failure is reader/version-specific and not a surface or metadata defect.

**C# passed this gate via a vendored fork:** `uniffi-bindgen-cs` v0.11.0 (and upstream `main`) targets uniffi 0.31 while the repo pins 0.32 — the stock `--library` metadata read fails, and the UDL fallback generates and compiles (net8.0, zero warnings/errors) but the runtime checksum gate rejects all 14 exported symbols before any call executes. The repo vendors a fork retargeted to uniffi 0.32: the generated binding passes the checksum gate and the golden-parity smoke (net8.0). The fork is dropped when a `uniffi-bindgen-cs` tag (or main commit) targets uniffi **0.32+**, re-checked via the regenerate → build → run sequence documented in `crates/spoke-connect/bindings/csharp/Smoke/README.md`. Go/Python bindgen tools get the same feasibility gate before their binding slices start. Full record: [`connect-csharp-binding.md`](../../specs/connect-csharp-binding.md).

### Next-slice binding checklist

1. [x] Freeze the stable sync core API list — **done**: 8 functions + 3 objects + the two error enums are the exported surface.
2. [x] Map `CoreError` / `CoreInvokeError` variants to foreign-language error enums — **done** for Swift (`TokenInvalid` included); the mapping table lives in the crate README "Binding facade" section.
3. [~] Choose the Path B shape for the first language — **core-only landed for Swift** (host-language transport + Rust core); the **core + async node** option (bridging a thin `SpokeConnectNode` via the uniffi async/foreign-runtime mechanism) stays open for a later iteration or a second language.
4. [~] Share the golden hello vector with every other language path — **TypeScript asserts the same constants byte-for-byte** (`src/golden.ts` in `@42ch/spoke-connect`); the **Swift smoke asserts the golden peer id + signature locally**; a shared cross-language fixture remains a follow-up.
5. [x] Keep `Multiaddr` / swarm types off the FFI — **done**: satisfied by the landed surface.

## Why This Matters

An FFI boundary is a compatibility contract: once a foreign language imports the sync core, every subsequent core change is a breaking change for that language. Keeping the core pure keeps the contract small, testable, and byte-stable; keeping it sync means the first binding needs no runtime bridge at all. The landed Swift skeleton validates both claims end-to-end: the same eight-function surface crossed a real FFI boundary with no generated schema types on the wire and no tokio on the foreign side, and its golden-vector checks prove byte parity from a second language. The golden-vector discipline makes byte parity across languages an executable assertion instead of a hope, and the module boundary prevents transport concerns (dial state, identify buffering, stream lifecycle) from leaking into the portable rules.

## When to Apply

- Preparing any Rust crate for a uniffi/bindgen boundary — extract the pure rules first, then freeze a minimal `Send + Sync` facade with private internals.
- Porting connect session rules to a new language (Path A) — port against the pure core's behavior and golden vectors, not against transport internals.
- Choosing what a first binding skeleton exposes — start with the sync session rules; leave the async node lifecycle for the slice that solves runtime bridging.
- Landing a first uniffi skeleton — export with proc-macros (no UDL), gate the surface behind a non-default feature + `cdylib`, keep generated bindings gitignored and scripted, and run a local-language smoke while CI exercises only the Rust surface.

## Examples

### The core surface (as re-exported from `crates/spoke-connect/src/core/`)

```rust
pub use allowlist::is_allowlisted;
pub use correlate::{check_response_correlation, Correlation};
pub use dispatch::{dispatch_allowed, required_capability, CAPABILITY_L2_COMPUTABLE, CAPABILITY_SPOKE_BASELINE};
pub use error::{CoreError, CoreInvokeError};
pub use hello_crypto::{sign_hello_ed25519, verify_hello_ed25519};
pub use nonce::NonceStore;
pub use peer_id::derive_peer_id_from_ed25519_pubkey;
pub use sequence::{InboundSequence, OutboundSequence, MAX_SEQUENCE};
```

Every item is pure, synchronous, and operates on plain data — the exact list a binding skeleton exports first. The transport side (node, session, runtime) remains crate-private behind the locked facade.

## See also

- [`spoke-connect-libp2p-spike.md`](spoke-connect-libp2p-spike.md) — the transport side of the same crate: minimal feature set, identify-key ↔ noise-PeerId binding, allowlist timing, pending-dial binding, `catch_unwind` handler containment, and the uniffi-ready facade preconditions (its "Event-loop + handler contract" bullet).
- [`spoke-connect-wire-and-auth.md`](spoke-connect-wire-and-auth.md) — the wire family, identity binding, and auth model this core implements.
- [`connect-identity-parity-proof.md`](../testing-patterns/connect-identity-parity-proof.md) — the cross-language byte-parity methodology the golden vectors feed.
- [`connect-ts-client-sdk.md`](connect-ts-client-sdk.md) — the Path A language-direct port (pure-TS) that mirrors this core's surface and asserts the same golden vectors.
- [`connect-capability-token-auth.md`](connect-capability-token-auth.md) — the step-up auth method whose `TokenInvalid` failure surfaces in the exported error enums.
- [`connect-csharp-binding.md`](../../specs/connect-csharp-binding.md) — C# binding decision record (landed via vendored bindgen fork; version gap, what was tried, drop-fork trigger).
- [`connect-uniffi-bindgen-fork.md`](connect-uniffi-bindgen-fork.md) — reusable vendored-fork technique when a community bindgen lags the repo uniffi pin (isolation, minimal patch, drop off-ramp).
