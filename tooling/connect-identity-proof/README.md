# connect-identity-proof

Throwaway Node proof that SPOKE connect identity bytes are reproducible in JavaScript.

**Not** a workspace package, **not** published, **not** CI-gated. Run locally before a TypeScript connect client slice.

## What it checks

Against golden vectors from the Rust pure session core (`crates/spoke-connect` `peer_id` + `hello_crypto`):

1. `peer_id` derivation (libp2p protobuf PublicKey → identity multihash `0x00` → base58btc)
2. RFC 8785 JCS UTF-8 bytes of the signed hello object
3. Ed25519 sign over those bytes (WebCrypto; PKCS8 import of the 32-byte seed)
4. base64url (no padding) of the 64-byte raw signature
5. Ed25519 verify of the golden signature

Normative rules: [`.mstar/specs/spoke-connect.md`](../../.mstar/specs/spoke-connect.md) §Identity binding and §Signature canonicalization.

## Run

Requires Node **≥ 20** with WebCrypto `Ed25519` (verified on Node 24). Zero npm dependencies.

```bash
node tooling/connect-identity-proof/proof.mjs
```

Exit `0` on full pass; non-zero on any mismatch.
