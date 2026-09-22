# connect-identity-proof

Throwaway Node proof that SPOKE connect identity bytes are reproducible in JavaScript.

Run locally with `node proof.mjs` (kept outside workspace packages). The proof also runs in CI as a regression gate — see [CI gate](#ci-gate).

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

## CI gate

The `connect-identity` job in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) runs the proof on Node 24 for every pull request and for pushes to `main`. It is a required PR check and therefore deliberately **not** path-filtered: a changed-path gate would report the context green without running the proof, which is indistinguishable from a real pass. A non-zero proof exit fails the job and the workflow.
