---
module: spoke-connect
date: 2026-08-06
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
plan_id: connect-demo-mvp
related_components: ["spoke-connect", "spoke-operations", "spoke-schemas"]
tags: [connect-responder, envelope-auth, public-exports, dispatch-gate, transport-seam, ts-interop]
applies_when: ["building a TypeScript responder or host that speaks protocol_version 2 connect", "reimplementing a protocol algorithm over public library primitives because the library keeps helpers module-internal", "hosting a BaselinePorts adapter behind a connect session with per-invoke dispatch gating", "integrating a third-party peer against the strict library client where canonical bytes must match exactly", "porting the responder recipe to another language against the Rust reference behavior"]
---

# Building a connect responder in TypeScript from public exports

## Context

`@42ch/spoke-connect` is a **client** library. Its protocol_version 2 per-envelope authentication helpers (`authenticateSession` / `verifySessionAuth` / `authenticateInvokeRequest` / `verifyInvokeRequestAuth` / `authenticateInvokeResponse` / `verifyInvokeResponseAuth` in `src/core/envelope-auth.ts`) are **module-internal by design** (spec D10 — consumers reach enforcement only through the RemoteAdapter surface). A third-party TypeScript **responder** (server-side host) still has to speak protocol_version 2 itself: it signs the `ConnectSession` snapshot it emits, verifies every inbound `ConnectInvokeRequest` before dispatch, and signs every `ConnectInvokeResponse`. Because the helpers are internal, the responder reimplements the three post-hello algorithm ids over the **public** primitives — the crypto and session-core exports from the package root plus the pinned RFC 8785 JCS canonicalizer.

This pattern is the recipe proven by the demo responder (`examples/connect-demo/server/src/host/` — `envelope-auth.ts`, `connect-host.ts`, `port-dispatch.ts`), which mirrors the library's canonical responder-phase reference (`packages/spoke-connect-ts/tests/remote/loopback-host.ts`). The same construction is normative in `.mstar/specs/spoke-connect.md` §Envelope authentication (protocol_version 2) and enforced by `.mstar/specs/spoke-remote-adapter.md` D10.

## Guidance

### 1. Reuse the public primitives; reimplement the algorithm ids

The responder's surface is the public package exports, split into two layers:

**Reused from `@42ch/spoke-connect` (root `.` subpath):**

- crypto: `signEd25519`, `verifyEd25519`, `base64UrlEncode`, `base64UrlDecode`, `getPublicKeyEd25519`
- identity: `derivePeerIdFromEd25519Pubkey`
- session-core: `signHelloEd25519` / `verifyHelloEd25519`, `generateNonce`, `NonceStore`, `isAllowlisted`, `negotiatedCapabilities`, `Session` (`peekInboundSequence` / `acceptInboundSequence`), `requiredCapability`, `dispatchAllowed`
- framing: `encodeJsonMessage` / `decodeJsonMessage`

**Reused from `@42ch/spoke-connect/remote`:** `isConnectHello`, `isConnectInvokeRequest`, and the message-oriented `Transport` type (one envelope per `send` / `recv` — the same seam the RemoteAdapter consumes).

**Reimplemented:** the three envelope-auth algorithm ids — the signed-object builders, the JCS canonicalization step, and the fail-closed verify sequence — over the public crypto primitives plus the **pinned** `canonicalize@3.0.0` package (RFC 8785 JCS). Both the library and the demo responder pin `canonicalize` at exactly `3.0.0`; JCS is the sole canonicalizer, so signer/verifier byte parity depends on the same canonicalizer version. A responder that signs with `JSON.stringify` produces different bytes (key order, number formatting) and the strict verifier rejects them as `envelope_auth_invalid`.

### 2. The three algorithm ids and their locked field sets

All four algorithm ids (hello unchanged plus the three post-hello ids) share one construction: build the signed object — **exact keys, no others** — → RFC 8785 JCS → UTF-8 bytes → Ed25519 sign/verify with the peer-identity key → base64url **without padding** (86 characters). `extensions` and `signature` are excluded from every signed object.

| Algorithm id | Envelope | Signed fields |
|--------------|----------|---------------|
| `spoke-connect-session-jcs-v1` | `ConnectSession` | `{session_id, initiator_peer_id, responder_peer_id, opened_at, negotiated_capabilities, initial_sequence}` |
| `spoke-connect-invoke-request-jcs-v1` | `ConnectInvokeRequest` | `{session_id, sequence, request_id, op, payload}` plus `auth` **when present** on the wire (trust-affecting; MUST be bound) |
| `spoke-connect-invoke-response-jcs-v1` | `ConnectInvokeResponse` | success branch `{session_id, sequence, request_id, payload}` XOR error branch `{session_id, sequence, request_id, error}` |

The two response branches are signed independently over their respective field sets — they are **never merged** into one signed object with optional `payload`/`error`; the discriminator is implicit in which keys are present.

Verify is fail-closed, in order (spec §Verify rules): presence → canonical base64url round-trip (`decode → encode` equality, rejecting padded input and non-canonical slack bits) → exact-keys signed-object construction (unknown key = field-set drift) → JCS → Ed25519 verify with the peer's hello public key → session binding. For responses, the correlation echo check (`session_id` / `sequence` / `request_id` vs the pending request) runs **before** the signature verify. Rejections carry `code: "auth_failed"` with a stable machine kind in `details.kind`: `envelope_auth_missing` / `envelope_auth_invalid` / `envelope_auth_session_unbound`.

### 3. The responder-phase recipe (mirror of loopback-host)

The responder runs two phases over the transport, exactly as the loopback host does:

**Handshake phase** — order matters, allowlist first:

1. `recv()` the initiator hello; shape-check with `isConnectHello`.
2. Allowlist check on `hello.peer_id` **first** — an untrusted peer is rejected before any signature work.
3. Verify the hello: derive `derivePeerIdFromEd25519Pubkey(clientPubkey)` and `verifyHelloEd25519(clientPubkey, clientPeerId, helloDoc)` — the hello `peer_id` binds to the preconfigured key.
4. Nonce single-use: `nonceStore.checkAndRecord(peer_id, nonce)` — a replay fails the handshake.
5. Build the local `Session` (`session_id`, `initiator_peer_id` = client peer id, `responder_peer_id` = host peer id, `negotiated_capabilities` = `negotiatedCapabilities(hostManifest.capabilities, helloDoc.host.capabilities)`).
6. Answer with the **responder hello** (`signHelloEd25519(seed, generateNonce(), manifest, helloDoc.nonce)`) — the 5-field object with `peer_nonce` = the initiator's nonce (dial binding: a captured responder hello is not replayable into a fresh dial).
7. Emit the signed `ConnectSession` snapshot (`authenticateSession` over the six locked fields, `initial_sequence: 0`, ≥1 negotiated capability) — the client's dial verifies it against the host's hello public key.

**Serve loop** — each inbound invoke passes a serialized gate, then dispatches:

1. `peekInboundSequence(sequence)` — non-mutating wire-position check (reject `inbound_sequence_mismatch` without side effects).
2. `verifyInvokeRequestAuth(clientPubkey, doc, session.session_id)` — fail-closed; a forged/tampered/stripped signature produces **no handler side effect and no session-state mutation**.
3. `acceptInboundSequence(sequence)` — advance the inbound counter **only after** envelope-auth verify passes.
4. Dispatch gate (see §4), then the op handler, then a signed response via `authenticateInvokeResponse` — success branch or error branch, never both.

Gate serialization: `peek → verify → advance` must run to completion per request (the async verify would otherwise let a second request peek against the pre-advance counter and be mis-rejected as `inbound_sequence_mismatch`). Queue invokes on a `gateTail` promise; the dispatch phase after the gate may interleave — out-of-order completions are legal on the wire.

```ts
// Gate phase — the serialized core of the serve loop (demo connect-host.ts).
async function runGate(doc: ConnectInvokeRequest): Promise<GateResult> {
  const current = session;
  if (current === null || doc.session_id !== current.session_id) {
    return null; // stray request — ignored
  }
  try {
    current.peekInboundSequence(doc.sequence);           // 1. non-mutating
  } catch {
    return { ok: false, code: "inbound_sequence_mismatch" };
  }
  try {
    await verifyInvokeRequestAuth(clientPubkey, doc, current.session_id); // 2. fail-closed
  } catch (error) {
    if (error instanceof EnvelopeAuthError) {
      return { ok: false, code: "auth_failed", details: { kind: error.kind } };
    }
    throw error; // wrong-length key is host misconfiguration — fail loudly
  }
  current.acceptInboundSequence(doc.sequence);           // 3. advance only after verify
  return { ok: true };
}
```

### 4. The dispatch-gate product map: `port.*` → `spoke-baseline`

The op dispatch gate runs before the handler: the required capability for `op` must be present in the session's `negotiated_capabilities`, else the responder answers the error branch (`op_unsupported`) with no side effects. For baseline **port-method** ops (D4 catalogue: `port.knowledge.get`, `port.relation.put`, `port.scope.list_*`, …), the core `requiredCapability` table returns `undefined` — the core table covers the ops-family vocabulary (`upsert`, `check`, …), not `port.*`. Without a product map, **every** `port.*` invoke would be denied `op_unsupported`.

The responder supplies its own product `op_capability_requirements` map — every baseline `port.*` op requires `spoke-baseline`:

```ts
export const PORT_OP_CAPABILITY_REQUIREMENTS: Record<string, string> = {
  "port.knowledge.get": "spoke-baseline",
  "port.knowledge.put": "spoke-baseline",
  "port.relation.get": "spoke-baseline",
  "port.relation.put": "spoke-baseline",
  "port.scope.list_knowledge_entries": "spoke-baseline",
  "port.scope.list_timeline_events": "spoke-baseline",
  "port.finding.put": "spoke-baseline",
  "port.rule.list": "spoke-baseline",
  "port.host.list_peer_manifests": "spoke-baseline",
};

// Dispatch gate — product map first, then the core table.
const required = requirements[doc.op] ?? requiredCapability(doc.op);
if (required === undefined || !current.negotiated_capabilities.includes(required)) {
  // answer the error branch: op_unsupported
}
```

The capability-token gate is policy-optional (spec: "when policy requires a valid token"); the reference hosts run the dispatch order sequence → envelope-auth → dispatch gate → handler without it.

### 5. Prove the responder with the strict library client

The end-to-end proof is a real `connectRemoteAdapter` (from `@42ch/spoke-connect/remote`) dialing the responder over a real WebSocket transport. The library enforces protocol_version 2 **strictly** on every post-hello envelope it emits or accepts (D10): the dial verifies the responder hello (`peer_nonce` dial binding) and the signed `ConnectSession` snapshot against the responder's hello public key; every inbound response is correlation-checked then signature-verified; every outbound invoke carries a signature the responder verifies before dispatch. A responder that shares the canonical bytes and the key discipline therefore interoperates out of the box, and **green tests equal interop**: a full port round-trip (put → get with OCC revisions, list, findings) through the real client is bidirectional signature interop proof — both directions verified by the strict side. The negative case proves the allowlist gate: a non-allowlisted stranger identity fails the dial (`ws connection closed`) before any session exists.

```ts
// The proof shape (demo e2e): real server, real WebSocket, real library client.
const server = await serveConnectDemo({ port: 0 });
const run = await runDemoClient({ url: server.url });
expect(run.remotePeerId).toBe(server.peerId);      // hello + session snapshot verified
expect(run.created.revision).toBe(1);              // signed invoke accepted by responder
expect(run.updated.revision).toBe(2);              // OCC compare-and-swap round-trip
expect(run.fetched).toEqual(run.updated);          // signed response verified by client
```

## Why This Matters

The enforcement boundary is deliberate: consumers reach envelope authentication only through the RemoteAdapter surface, so a third-party responder has no shared helper to import — it must reimplement the three algorithm ids from the spec over public primitives. That is exactly where interop is won or lost: the strict library client rejects anything that is not the exact canonical bytes (JCS with the pinned canonicalizer, canonical base64url, locked field sets), so any field-set drift, order dependence, or decoder slack on the responder side surfaces as `auth_failed` — visible, but only if the responder is tested against the real client. The demo responder proves the recipe end to end, and its shape doubles as the porting reference for other languages against the Rust reference behavior.

## When to Apply

- Building a TypeScript responder or host that speaks protocol_version 2 connect (server side of the wire).
- Reimplementing a protocol algorithm over public library primitives when the library keeps helpers module-internal by design.
- Hosting a `BaselinePorts` adapter behind a connect session with per-invoke dispatch gating.
- Integrating a third-party peer against the strict library client, where canonical bytes must match exactly.
- Porting the responder recipe to another language against the Rust reference behavior.

## Examples

### Before: order-dependent signing (rejected by the strict verifier)

```ts
// JSON.stringify output is NOT the canonical bytes — key order and number
// formatting differ from RFC 8785 JCS, so the strict verifier rejects the
// envelope as envelope_auth_invalid.
const signature = await signEd25519(seed, new TextEncoder().encode(
  JSON.stringify({ session_id, sequence, request_id, op, payload }),
));
```

### After: signed-object construction over public primitives

```ts
import canonicalize from "canonicalize"; // pinned 3.0.0 — the sole canonicalizer
import {
  base64UrlDecode, base64UrlEncode,
  signEd25519, verifyEd25519,
} from "@42ch/spoke-connect";

const SESSION_SIGNED_KEYS = [
  "session_id", "initiator_peer_id", "responder_peer_id",
  "opened_at", "negotiated_capabilities", "initial_sequence",
];

function sessionSignedObject(session: SessionSignInput): Record<string, unknown> {
  return {
    session_id: session.session_id,
    initiator_peer_id: session.initiator_peer_id,
    responder_peer_id: session.responder_peer_id,
    opened_at: session.opened_at,
    negotiated_capabilities: session.negotiated_capabilities,
    initial_sequence: session.initial_sequence,
  }; // exact keys, no others — extensions and signature excluded
}

async function signEnvelope(
  secret: Uint8Array,
  signedObject: Record<string, unknown>,
): Promise<string> {
  const jcs = canonicalize(signedObject);
  if (jcs === undefined) {
    throw new EnvelopeAuthError("envelope_auth_invalid", "signed object is not JSON-serializable");
  }
  return base64UrlEncode(await signEd25519(secret, new TextEncoder().encode(jcs)));
}

async function authenticateSession(session: SessionSignInput): Promise<ConnectSession> {
  const signature = await signEnvelope(seed, sessionSignedObject(session));
  return { ...session, signature, extensions: {} }; // wire envelope
}
```

### After: signed response branches, never merged

```ts
async function authenticateInvokeResponse(
  response: InvokeResponseSignInput,
): Promise<ConnectInvokeResponse> {
  if ("payload" in response) {
    const signature = await signEnvelope(seed, {
      session_id: response.session_id,
      sequence: response.sequence,
      request_id: response.request_id,
      payload: response.payload, // success branch — its own signed object
    });
    return { ...response, signature, extensions: {} };
  }
  const signature = await signEnvelope(seed, {
    session_id: response.session_id,
    sequence: response.sequence,
    request_id: response.request_id,
    error: response.error, // error branch — signed independently
  });
  return { ...response, signature, extensions: {} };
}
```

## See also

- `encapsulated-remote-adapter-bridge.md` — the client/RemoteAdapter side of the same wire (port-method invoke mapping, Transport seam, strict v2 enforcement)
- `spoke-connect-wire-and-auth.md` — wire family, hello signature canonicalization, identity binding, and the auth vocabulary the responder builds on
- `connect-session-core-ffi-boundary.md` — the pure session-core extraction whose `Session` helpers (`peekInboundSequence` / `acceptInboundSequence`) the responder reuses
- `.mstar/specs/spoke-connect.md` §Envelope authentication (protocol_version 2) — normative field sets and verify rules
- `.mstar/specs/spoke-remote-adapter.md` D4 + D10 — port-method catalogue and the module-internal enforcement boundary
