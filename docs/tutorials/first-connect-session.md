---
title: Open your first connect session
---

# Open your first connect session

This tutorial establishes a SPOKE Connect session end-to-end: derive your `peer_id`, sign a hello, verify the remote's signed hello against an allowlist, and invoke an op with correlation. It uses the TypeScript **language-native client** (`@42ch/spoke-connect`) against a local peer, and points to the Rust reference crate for the peer side.

Connect is the opt-in interaction envelope family (`spoke-connect` capability flag) for cross-process SPOKE hosts. You should have completed [Install and create your first KnowledgeEntry](/tutorials/install-and-first-entry) first — this tutorial builds identity and session concepts on top of the data/ops story.

## 1. Install the client

```bash
pnpm add @42ch/spoke-connect@X.Y.Z
```

`@42ch/spoke-connect` exports four entry points:

- **`.`** — the isomorphic core: identity derivation, Ed25519 crypto, RFC 8785 JCS canonicalization, and the pure session-core rules (allowlist, nonce, sequence, correlation, dispatch gate).
- **`./node`** — the Node `connectClient`, which dials a WebSocket and performs the full handshake.
- **`./noise`** — the opt-in Noise XX mesh transport subpath for direct libp2p-noise interoperability.
- **`./remote`** — the opt-in RemoteAdapter module: `connectRemoteAdapter` over a consumer `Transport`, the multi-peer router, and the in-repo loopback pair (see [RemoteAdapter over a Transport](/how-to/connect-remote-adapter)).

## 2. Derive your peer identity

Every connect host has an Ed25519 keypair. The wire `peer_id` is derived from the 32-byte public key — the identity multihash of the libp2p `PublicKey` protobuf, base58btc-encoded. The derivation is byte-identical across the TypeScript client, the Rust reference, and all native bindings (locked by shared golden vectors).

```ts
import { derivePeerIdFromEd25519Pubkey, getPublicKeyEd25519 } from "@42ch/spoke-connect";

const seed = new TextEncoder().encode("..."); // 32-byte Ed25519 seed
const publicKey = getPublicKeyEd25519(seed);
const peerId = derivePeerIdFromEd25519Pubkey(publicKey);

console.log(peerId); // base58btc, e.g. 12D3KooW...
```

`peer_id` is the network trust root — distinct from the advisory `host_id` carried inside the manifest.

## 3. Sign and verify a hello

The handshake is a signed `ConnectHello`: the object `{protocol_version, peer_id, nonce, host}` (initiator hello) — or `{protocol_version, peer_id, nonce, host, peer_nonce}` with `peer_nonce` = the initiator's nonce (responder hello, dial binding) — is canonicalized with RFC 8785 JCS ([RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)), signed with Ed25519, and the raw signature is encoded base64url without padding ([RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648)).

```ts
import { generateNonce, signHelloEd25519, verifyHelloEd25519 } from "@42ch/spoke-connect";
import type { HostCapabilityManifest } from "@42ch/spoke-schemas";

const manifest: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "host_tutorial",
  roles: ["input-source"],
  capabilities: ["spoke-baseline"],
  namespaces: ["tutorial"],
  extensions: {},
};

const nonce = generateNonce(); // 16 CSPRNG bytes, base64url — ≥ the 16-char wire floor
const hello = await signHelloEd25519(seed, nonce, manifest);

// On the receiving side: verify against the sender's public key AND its
// derived peer id — a key that derives a different peer id cannot attest
// that peer's identity.
await verifyHelloEd25519(remotePubkey, remotePeerId, hello);
```

Nonces are single-use per sender: the receiver records each accepted `(peer_id, nonce)` pair in a `NonceStore` and rejects replays.

The **responder** (the side that received a hello) signs its own hello with the initiator's nonce — the dial binding. The initiator passes its own nonce into verification, so a captured responder hello cannot be replayed into a fresh dial:

```ts
// Responder: echo the initiator's nonce into the signed object (5 fields).
const responderHello = await signHelloEd25519(seed, generateNonce(), manifest, receivedHello.nonce);

// Initiator: assert the responder's peer_nonce equals our own nonce.
await verifyHelloEd25519(remotePubkey, remotePeerId, responderHello, ourNonce);
```

## 4. Configure the allowlist

Admission is fail-closed: an empty allowlist rejects every peer. The receiving host accepts a connection only when the authenticated remote `peer_id` is listed.

```ts
import { isAllowlisted, NonceStore } from "@42ch/spoke-connect";

const allowlist = [remotePeerId];
if (!isAllowlisted(allowlist, remotePeerId)) {
  throw new Error(`peer ${remotePeerId} is not allowlisted`);
}

const nonceStore = new NonceStore();
nonceStore.checkAndRecord(remotePeerId, hello.nonce); // false on replay
```

## 5. Sequence and correlation

Each session maintains per-direction monotonic `sequence` counters starting at 0. Every invoke attaches a `request_id`; the response must echo `session_id`, `sequence`, and `request_id` or the correlation check fails.

```ts
import { OutboundSequence, checkResponseCorrelation, correlationFromRequest, correlationFromResponse } from "@42ch/spoke-connect";

const outbound = new OutboundSequence();
const request = {
  session_id: "sess_1",
  sequence: outbound.allocate(), // first call → 0
  request_id: crypto.randomUUID(),
  op: "check",
  payload: { scope: { scope_id: "book-harbor" } },
  extensions: {},
};

// When the response arrives:
checkResponseCorrelation(correlationFromRequest(request), correlationFromResponse(response));
```

## 6. Full session: `connectClient`

`connectClient` (from the `./node` subpath) dials a WebSocket, performs the signed hello exchange, validates the session snapshot (peer binding, `initial_sequence` 0), and routes correlated invokes by `request_id`:

```ts
import { derivePeerIdFromEd25519Pubkey } from "@42ch/spoke-connect";
import { connectClient } from "@42ch/spoke-connect/node";

const remotePubkey = /* the peer's 32-byte Ed25519 public key */;

const client = await connectClient({
  url: "ws://127.0.0.1:8080",
  identity: { seed },
  manifest,
  remotePubkey,
  allowlist: [derivePeerIdFromEd25519Pubkey(remotePubkey)],
});

const response = await client.invoke("check", { scope: { scope_id: "book-harbor" } });
client.close();
```

The client rejects before the handshake starts when the remote peer id is missing from the allowlist, and rejects the session when the snapshot's peer ids do not match the authenticated hellos.

## 7. The peer side

`connectClient` connects to any SPOKE host that speaks the connect wire family over an ordered reliable stream. The **Rust reference crate** (`spoke-connect` on crates.io) is the reference host implementation — it maps the envelopes onto rust-libp2p (noise, yamux, request-response) and demonstrates a two-node session where one node dials the other, exchanges signed hellos, and invokes `check`:

```bash
cargo add spoke-connect@X.Y.Z
cargo run -p spoke-connect --example two_node_usage
```

The compiled example source ([`examples/two_node_usage.rs`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/examples/two_node_usage.rs)) shows both sides: `SpokeConnectNode::start` with `peer_allowlist` and a local manifest, then `connect(addr)` and `session.invoke("check", payload)` — the same session rules the TypeScript client implements. The crate README ([`crates/spoke-connect/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md)) documents the full flow, including capability-token step-up auth.

## What you now know

- `peer_id` derivation from an Ed25519 public key, and why it is the trust root.
- The signed hello (`spoke-connect-hello-jcs-v1`): JCS over `{protocol_version, peer_id, nonce, host}` (initiator) / plus `peer_nonce` (responder), Ed25519 signature, base64url, and the dial binding that rejects replayed responder hellos.
- Fail-closed allowlist admission and single-use nonce replay protection.
- Per-session sequence and `request_id` correlation.

## Next steps

- [Integrate a RemoteAdapter against a live host](/tutorials/integrate-remote-adapter) — implement a `Transport`, dial with `connectRemoteAdapter`, and call the `BaselinePorts` surface against the demo mock inference host.
- [Connect from the TypeScript client](/how-to/connect-ts-client) — the full client surface, browser vs Node, and core helpers.
- [Connect from native bindings](/how-to/connect-native-bindings) — the same session core from C#, Kotlin, Swift, Go, or Python.
- [Connect wire reference](/reference/connect) — envelope field tables and identity binding rules.
