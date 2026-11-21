---
module: spoke-connect
date: 2026-08-01
last_updated: 2026-09-22
problem_type: testing_pattern
category: testing-patterns
severity: high
applies_when: ["porting connect identity or hello-signing logic to a new language", "adding a new language SDK target for spoke-connect", "refactoring the session core while preserving wire bytes"]
tags: [spoke-connect, identity-parity, golden-vectors, peer-id, jcs, ed25519, cross-language, webcrypto]
---

# connect identity-byte parity across languages

## Context

Connect identity bytes must be **byte-identical** across every language target: the Rust reference signs hellos and derives `peer_id`s, the TypeScript route must reproduce the same bytes in browser and Node, and the native bindings (Swift, Kotlin, C#, Go, Python) re-expose the Rust core. The hello signature scheme (`spoke-connect-hello-jcs-v1`) uses RFC 8785 JCS precisely so that one deterministic byte sequence — canonical JSON → UTF-8 bytes → Ed25519 sign → base64url — is portable. Parity is not a nice-to-have: a single-byte divergence in the canonical bytes breaks signature verification between peers implemented in different languages.

## Guidance

### Golden vectors are the cross-language contract

Define one fixed input and one set of committed outputs; every language implementation asserts against them:

| Item | Golden value (seed bytes 1..=32) |
|---|---|
| Public key | `79b5562e8fe654f94078b112e8a98ba7901f853ae695bed7e0e3910bad049664` |
| `peer_id` | `12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf` |
| JCS UTF-8 bytes | 264 bytes (hex committed in the Rust core tests and the TS fixture loader) |
| Signature (base64url, no pad) | `yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg` |

Capture vectors **before** refactoring — from the working implementation (e.g. rust-libp2p `PeerId::to_string()` / `Keypair::sign` over `serde_jcs`) — then assert against the committed constants; never assert against values the code under test computes itself.

### Keeper: the package golden-vector test

The parity check lives in the package test suite:

- `packages/spoke-connect-ts/tests/golden.test.ts` asserts `peer_id`, JCS UTF-8 bytes, and the signature against the committed constants, on both the WebCrypto and `@noble/ed25519` paths.
- Fixtures load through `packages/spoke-connect-ts/tests/fixtures/golden.ts` from the registered copy of the cross-language SSOT (`tests/fixtures/golden-hello.json`; sync gate: `tooling/connect/golden-vector-sync.mjs`).
- Runs under the CI `typescript` job (`pnpm run ci:typescript`) and locally with `pnpm -F @42ch/spoke-connect exec vitest run tests/golden.test.ts`.
- Re-run it before a TypeScript connect client slice; the same vectors are the regression check for any future pure-TS or uniffi implementation.

### Gotchas (the non-obvious parts that break byte parity)

1. **Protobuf `PublicKey` encoding — not raw bytes.** `peer_id` derives from the libp2p protobuf `PublicKey{Type=Ed25519, Data=pubkey}` message (36 bytes: `0x08 0x01 0x12 0x20 || key`), not from the 32-byte raw key alone.
2. **Identity multihash `0x00` — not sha2-256.** The digest is the protobuf bytes themselves (`0x00 0x24 || pk_bytes`); hashing the raw pubkey with sha2-256 produces a different, non-Ed25519-shaped PeerId.
3. **base58btc only** — no multibase prefix character (no leading `z`), no CIDv1 wrapping.
4. **JCS optional-field omission.** An absent optional field (`authority`) must be **omitted**, not serialized as `null` — the generated Rust manifest omits `None` via `skip_serializing_if`; emitting `"authority": null` changes the canonical bytes and breaks verification against Rust.
5. **`JSON.stringify` surrogate pairs.** Trivial JS string encoding can diverge from JCS for non-ASCII characters; the ASCII golden vectors are safe for the pure-JS subset, but real payloads with non-ASCII content should use a maintained JCS implementation.
6. **base64url without padding.** Encode the 64 raw signature bytes as base64url and strip `=`; padding changes the wire `signature` value.
7. **WebCrypto import formats.** WebCrypto needs DER-wrapped keys: PKCS8 for the 32-byte seed, SPKI for the 32-byte public key; the TypeScript client hand-encodes the RFC 8410 headers (`ed25519SeedToPkcs8`, `ed25519PubkeyToSpki`) instead of pulling in a PEM library.

## Why This Matters

Every connect implementation — Rust reference, pure-TS client, native bindings — must agree byte-for-byte on `peer_id`, canonical hello bytes, and signatures. Golden vectors turn that requirement into an executable check per language; one package test keeps the check cheap enough to run before any port. The gotchas above are exactly where naive ports diverge: hashing instead of identity multihash, raw keys instead of protobuf, `null` instead of omission, padded base64url.

## When to Apply

- Any new language SDK target for connect — uniffi (Swift, Kotlin, …), Go, or a second Path A port.
- Refactoring or extracting the session core — re-run the parity checks after the change.
- Bridging WebCrypto or other platform crypto APIs that require DER-wrapped keys.

## Examples

### Keeper test run

```text
$ pnpm -F @42ch/spoke-connect exec vitest run tests/golden.test.ts

 Test Files  1 passed (1)
      Tests  8 passed (8)
```

The assertions cover the golden public key, `peer_id`, the 264-byte JCS object, the 64-byte signature and its base64url encoding, verification, and the auto-selected (WebCrypto-first) sign/verify path.

### Byte layout of the derivation (identical in every language)

```text
pubkey (32 bytes)
  → protobuf PublicKey (Ed25519): 0x08 0x01 0x12 0x20 || pubkey   (36 bytes)
  → identity multihash:           0x00 0x24 || pk_bytes          (38 bytes)
  → base58btc (no multibase prefix, no CIDv1):
      "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf"
```

## See also

- [`2026-08-01-spoke-connect-ts-route.md`](../../adr/implemented/architecture/2026-08-01-spoke-connect-ts-route.md) — the decision record for the pure-TS-minimal route these golden vectors pin.
- [`spoke-connect-wire-and-auth.md`](../architecture-patterns/spoke-connect-wire-and-auth.md) — the wire family and identity-binding model.
- [`connect-session-core-ffi-boundary.md`](../architecture-patterns/connect-session-core-ffi-boundary.md) — the pure-core extraction that hosts the Rust golden vectors.
