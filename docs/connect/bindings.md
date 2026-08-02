---
title: Native bindings (Path B)
---

# Native bindings (Path B)

Path B embeds a **shared session core** into host languages through FFI: the pure, synchronous session rules (`peer_id` derivation, hello sign/verify, allowlist, nonce single-use, sequence allocation, correlation, dispatch gate) live in one core, while transport stays in each host language. The reference implementation is the `spoke-connect` crate's **Binding facade**.

## Binding facade

The crate's `src/core/` is the pure, synchronous, language-portable layer — libp2p, tokio, and I/O live in the transport layer, which converts `libp2p::PeerId` ↔ `String` at the boundary. The facade decision is recorded in the crate README's [Binding facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) section, including the sync-vs-async boundary and the exported surface.

## Swift sync-core binding

A **Swift sync-core binding** ships through uniffi behind the optional `ffi` feature (`cdylib`): exported functions like `derivePeerIdFromEd25519Pubkey`, `signHelloEd25519` / `verifyHelloEd25519`, `isAllowlisted`, `checkResponseCorrelation`, `dispatchAllowed`, plus `NonceStore` / `OutboundSequence` / `InboundSequence` objects — with a macOS-local smoke asserting golden-vector parity. Async node lifecycle (start, listen, `connect`) stays Rust-side.

## Target matrix

C#, Go, Python, Swift, Kotlin are the target languages (priority per product direction); Swift is the first shipped binding. TypeScript is a parallel **Path A** track (language-direct). C# binding is pending `uniffi-bindgen-cs` support for uniffi 0.32; the target matrix keeps C# as the first target.

## Integrator notes

- **Core-only** — the exported surface is the session core; host languages implement their own transport adapter against the wire contract.
- Keys cross the FFI boundary as raw bytes; peer ids as strings; manifests / hellos as JSON strings.
- **Build-time generation** — binding code is generated from the crate's cdylib; this page documents the exported surface.

## Normative references

- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) §Embedding model — Path B definition and purity rules
- [crates/spoke-connect/README.md#binding-facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) — sync/async boundary, exported surface, target-language matrix
- [connect-csharp-bindgen-deferred.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-bindgen-deferred.md) — C# binding decision record
