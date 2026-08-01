---
module: spoke-connect
date: 2026-08-01
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["extracting a pure session core for cross-language binding", "preparing a Rust crate for a uniffi FFI boundary", "deciding sync vs async surfaces on a foreign-language facade"]
tags: [spoke-connect, session-core, ffi-boundary, uniffi, path-b, sync-async, golden-vectors]
---

# connect session-core extraction and FFI facade boundary

## Context

The connect embedding model (`.mstar/specs/spoke-connect.md` §Embedding model) offers **Path B — shared core bindings**: export a session-core implementation (Rust via uniffi) into a foreign host language, keeping the transport adapter in the host language or in the shared core. Before any binding can ship, the session rules must exist as a **pure, transport-free module** — otherwise every language target drags in a tokio/libp2p runtime. The `crates/spoke-connect` reference stack therefore extracted `src/core/` as the language-portable session rules; the same module doubles as a behavioral reference for **Path A** ports (language-direct implementations) that must match its accept/reject outcomes without sharing code. The first binding surface is deliberately **synchronous**: bridging a tokio runtime across an FFI boundary is the highest-risk part of the design, so the async node surface is deferred to a later slice instead of blocking the sync core.

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
| Node lifecycle (start / listen / shutdown), `connect`, invoke wait | **Async** (tokio) | Never block foreign UI threads on the swarm loop; a later slice chooses host-language transport + Rust core-only bindings (preferred) vs a uniffi async/foreign-runtime bridge |
| `invoke_handler` hook | Sync **on the event loop** today | Product handlers must return promptly and must not block on I/O; before exposing handlers over FFI, dispatch moves off the loop (`spawn_blocking`) |

Keys cross the boundary as raw bytes (`&[u8; 32]`), peer ids as strings, payloads as `serde_json::Value` — the first skeleton needs no `Multiaddr` / swarm types on FFI.

### Target-language matrix

Order binding targets by uniffi maturity and product value; record the decision in the crate README so the next slice starts from a settled surface:

| Language | Embedding path | Priority |
|---|---|---|
| Swift (iOS / macOS) | Path B uniffi | **First** — validates the sync core without browser crypto constraints |
| Kotlin (Android) | Path B uniffi | Second — same uniffi pipeline after the core stabilizes |
| C# | Path B uniffi | Later — secondary desktop/server hosts |
| Python | Path B uniffi | Later — async FFI / asyncio is historically finicky; core-only first |
| TypeScript (browser / Node) | **Path A** language-direct | Parallel track — no uniffi/WASM assumed; decided by the TS route |

### Next-slice binding checklist

1. Freeze the stable sync core API list (string peer ids, byte-oriented keys).
2. Map `CoreError` / `CoreInvokeError` variants to foreign-language error enums.
3. Choose the Path B shape for the first language: **core-only** (host-language transport; preferred default) vs **core + async node**.
4. Share the golden hello vector (JCS bytes + signature + `peer_id`) with every other language path so all agree byte-for-byte.
5. Keep `Multiaddr` / swarm types off the FFI in the first skeleton.

## Why This Matters

An FFI boundary is a compatibility contract: once a foreign language imports the sync core, every subsequent core change is a breaking change for that language. Keeping the core pure keeps the contract small, testable, and byte-stable; keeping it sync means the first binding needs no runtime bridge at all. The golden-vector discipline makes byte parity across languages an executable assertion instead of a hope, and the module boundary prevents transport concerns (dial state, identify buffering, stream lifecycle) from leaking into the portable rules.

## When to Apply

- Preparing any Rust crate for a uniffi/bindgen boundary — extract the pure rules first, then freeze a minimal `Send + Sync` facade with private internals.
- Porting connect session rules to a new language (Path A) — port against the pure core's behavior and golden vectors, not against spike internals.
- Choosing what a first binding skeleton exposes — start with the sync session rules; leave the async node lifecycle for the slice that solves runtime bridging.

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

### Environment quirk: nightly-only cargo rustflags on a stable toolchain

The machine-local `~/.cargo/config.toml` sets `[build] rustflags = ["-Zno-embed-metadata"]` — a nightly-only flag. On the stable toolchain cargo rejects `-Z` options, so local builds fail unless the override is applied:

```bash
RUSTFLAGS="" cargo build -p spoke-connect
```

Keep the override in the documented dev workflow until the config file is updated to match the stable toolchain.

## See also

- [`spoke-connect-libp2p-spike.md`](spoke-connect-libp2p-spike.md) — the transport side of the same crate: minimal feature set, identify-key ↔ noise-PeerId binding, allowlist timing, pending-dial binding, `catch_unwind` handler containment, and the uniffi-ready facade preconditions (its "Event-loop + handler contract" bullet).
- [`spoke-connect-wire-and-auth.md`](spoke-connect-wire-and-auth.md) — the wire family, identity binding, and auth model this core implements.
- [`connect-identity-parity-proof.md`](../testing-patterns/connect-identity-parity-proof.md) — the cross-language byte-parity methodology the golden vectors feed.
