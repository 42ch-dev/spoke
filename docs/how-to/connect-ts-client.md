---
title: Connect from the TypeScript client
---

# Connect from the TypeScript client

The **language-native client** (`@42ch/spoke-connect`) implements the connect wire contract and session-core rules in TypeScript: `peer_id` derivation, Ed25519 hello signing, RFC 8785 JCS canonicalization, one-JSON-per-message WebSocket framing, and the pure session-core rules (allowlist, nonce, sequence, correlation, dispatch gate, capability tokens). It pairs with the platform WebSocket — no Rust runtime involved.

```bash
pnpm add @42ch/spoke-connect@X.Y.Z
```

Two entry points:

- **`.`** — the isomorphic core: identity, crypto, JCS, and session core. Works in browsers and Node.
- **`./node`** — the Node `connectClient` (depends on `ws`), which dials a WebSocket and completes the handshake.

## Identity

```ts
import { derivePeerIdFromEd25519Pubkey, ed25519PubkeyFromPeerId, getPublicKeyEd25519 } from "@42ch/spoke-connect";

const publicKey = getPublicKeyEd25519(seed);            // 32-byte Ed25519 public key
const peerId = derivePeerIdFromEd25519Pubkey(publicKey); // wire peer_id (base58btc)
const roundTrip = ed25519PubkeyFromPeerId(peerId);       // reverse derivation
```

The derivation formula is the protocol identity binding: protobuf `PublicKey` → identity multihash `0x00` → base58btc. Byte parity with the Rust reference and all native bindings is locked by shared golden vectors.

## Crypto and JCS

```ts
import { base64UrlEncode, signEd25519, verifyEd25519, webcryptoEd25519Available } from "@42ch/spoke-connect";

const bytes = new TextEncoder().encode("...");
const signature = await signEd25519(seed, bytes);
const ok = await verifyEd25519(publicKey, bytes, signature);
```

Ed25519 uses WebCrypto where available with an `@noble/ed25519` fallback on the same code path (`webcryptoEd25519Available()` reports which path is active). Signatures are encoded base64url without padding ([RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648)).

`canonicalHelloBytes(peerId, nonce, host)` produces the RFC 8785 JCS bytes ([RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)) of the signed hello object `{protocol_version, peer_id, nonce, host}` — absent optional members are omitted from the canonical object.

## Signed hello and replay protection

```ts
import { generateNonce, signHelloEd25519, verifyHelloEd25519, NonceStore } from "@42ch/spoke-connect";

const nonce = generateNonce(); // 16 CSPRNG bytes, base64url
const hello = await signHelloEd25519(seed, nonce, manifest);

const store = new NonceStore();
store.checkAndRecord(remotePeerId, hello.nonce); // false when the (peer_id, nonce) pair was already accepted
await verifyHelloEd25519(remotePubkey, remotePeerId, hello);
```

The nonce floor is 16 characters; `signHelloEd25519` enforces it (`invalid_nonce`). The `NonceStore` records only accepted hellos, so a hello rejected by an earlier gate stays retry-safe.

## Allowlist and dispatch

```ts
import { isAllowlisted, dispatchAllowed, requiredCapability, CAPABILITY_SPOKE_BASELINE } from "@42ch/spoke-connect";

isAllowlisted(["12D3KooW..."], peerId);         // fail-closed: empty allowlist rejects all
dispatchAllowed("check", ["spoke-baseline"]);   // core-op capability ⊆ negotiated capabilities
requiredCapability("check");                    // "spoke-baseline" for core ops; null for product ops
```

The dispatch gate maps core ops to required capabilities (`upsert`, `promote`, `relate`, `check`, `assemble`, `project`, `compute`) and fails closed on unknown ops.

## Capability tokens

```ts
import { issueCapabilityToken, verifyCapabilityToken, TOKEN_VERSION, CLOCK_SKEW_SECONDS } from "@42ch/spoke-connect";

const proof = await issueCapabilityToken(issuerSeed, {
  iss: issuerPeerId,            // derived from issuerSeed's public key
  sub: subjectPeerId,           // who may present the token
  aud: verifierPeerId,          // the verifying node's peer_id
  capabilities: ["spoke-baseline"],
  exp: Math.floor(Date.now() / 1000) + 60,
  iat: Math.floor(Date.now() / 1000),
});

const granted = await verifyCapabilityToken(
  proof,
  [issuerPeerId],               // trusted issuers (fail-closed)
  thisPeerId,                   // this node's peer_id (aud check)
  sessionPeerId,                // the authenticated session peer
  Math.floor(Date.now() / 1000),
);
// granted = the validated capability list for the dispatch gate
```

Capability tokens are offline-validated, capability-scoped grants: a trusted issuer signs a short claim set (`iss` / `sub` / `aud` / `capabilities` / `exp`, optional `iat` / `jti`) over JCS with Ed25519, and the proof rides the auth challenge/response exchange or per-invoke `auth`. Verification enforces issuer trust, subject/audience binding, expiry, and clock skew (`CLOCK_SKEW_SECONDS`).

## Session state

```ts
import { Session, negotiatedCapabilities, OutboundSequence, InboundSequence, checkResponseCorrelation } from "@42ch/spoke-connect";

const session = new Session({
  session_id: "sess_1",
  initiator_peer_id: localPeerId,
  responder_peer_id: remotePeerId,
  negotiated_capabilities: negotiatedCapabilities(localCaps, remoteCaps),
});

session.allocateOutboundSequence(); // 0, 1, 2, … — no wrap past 2^53−1
```

Per-session, per-direction `sequence` counters start at 0; exhaustion closes the session and opens a new one. Responses echo `session_id` / `sequence` / `request_id` — `checkResponseCorrelation` enforces the match. `negotiatedCapabilities` computes the agreed subset of both hosts' capability lists.

## End-to-end with `connectClient`

The Node client performs the full flow — dial, signed hello exchange, session snapshot validation, correlated invokes:

```ts
import { derivePeerIdFromEd25519Pubkey } from "@42ch/spoke-connect";
import { connectClient } from "@42ch/spoke-connect/node";

const client = await connectClient({
  url: "ws://127.0.0.1:8080",
  identity: { seed },
  manifest: {
    schema_version: 1,
    host_id: "host_primary",
    roles: ["data-store"],
    capabilities: ["spoke-baseline"],
    namespaces: ["toy_world"],
    extensions: {},
  },
  remotePubkey,
  allowlist: [derivePeerIdFromEd25519Pubkey(remotePubkey)],
});

const response = await client.invoke("check", { scope: { scope_id: "book-harbor" } });
client.close();
```

The client rejects before dialing when the remote peer id is missing from the allowlist, and every handshake and invoke await is bounded by `timeoutMs` (default 5000).

## Browsers vs Node

The core imports (`@42ch/spoke-connect`) are browser-swappable — the Node client and its `ws` dependency stay behind the `./node` subpath. Browser consumers import the core only and pair it with the native WebSocket.

## Peer-side interoperability

The TypeScript client speaks the same session-core rules as the Rust reference (`spoke-connect` on crates.io) and every native binding — see [Connect wire reference](/reference/connect) for the shared contract. For a Rust-side peer, use `cargo add spoke-connect` and follow the two-node example in the [crate README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md).

## Next steps

- [Open your first connect session](/tutorials/first-connect-session) — the concepts behind each helper, step by step.
- [Connect from native bindings](/how-to/connect-native-bindings) — the same session core from C#, Kotlin, Swift, Go, or Python.
- [Connect wire reference](/reference/connect) — envelope field tables and identity binding.
