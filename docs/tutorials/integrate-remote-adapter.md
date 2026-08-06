---
title: Integrate a RemoteAdapter against a live host
---

# Integrate a RemoteAdapter against a live host

This tutorial connects a **RemoteAdapter** to a live SPOKE connect host over a real WebSocket: you run the demo mock inference host, implement the message-oriented `Transport` the adapter dials through, call the drop-in `BaselinePorts` surface, watch the host's inference engine derive artifacts from your data, and see how failures surface. The code you work with is the shipped demo in `examples/connect-demo/` — the same integration surface a third-party TypeScript app would use: the **language-native TypeScript client** `@42ch/spoke-connect` (its `./remote` subpath), the `@42ch/spoke-schemas` wire types, and a WebSocket library. Nothing else.

You should have completed [Open your first connect session](/tutorials/first-connect-session) first — this tutorial uses identities, allowlists, signed hellos, and sessions through the library instead of re-teaching them.

## 1. Meet the demo host

The demo is two packages under `examples/connect-demo/`:

- `server/` (`@42ch/spoke-demo-server`) — a deterministic **mock inference host**: a `BaselinePorts` adapter backed by a pure rule-based engine, served by a spec-faithful connect responder over a `ws` WebSocketServer.
- `client/` (`@42ch/spoke-demo-client`) — the **third-party story**: its own `Transport` implementation over `ws`, then the real library client (`connectRemoteAdapter` from `@42ch/spoke-connect/remote`) dials the host and calls the drop-in async `BaselinePorts` surface.

The host's identity and capabilities come from its manifest. The demo server advertises itself as `demo-inference-host` with the baseline capability and a single namespace, `demo-harbor` (`examples/connect-demo/server/src/adapter/mock-adapter.ts`):

```ts
export const DEMO_SERVER_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-inference-host",
  roles: ["checker", "assembler"],
  capabilities: ["spoke-baseline"],
  namespaces: [DEMO_SCOPE_ID],
  extensions: {},
};
```

`DEMO_SCOPE_ID` is the demo namespace, `"demo-harbor"` — every seed entity and every demo manifest belongs there. Behind the manifest sits `MockEngine`, a deterministic inference engine: it starts from a fixed seed corpus (two KnowledgeEntries — Mira the dockworker and the Harbor district — plus one relation and one rule), accepts conditional puts with optimistic concurrency, and re-derives its own artifacts after every accepted mutation. Derivation is a pure function of store history: no wall clock, no randomness.

The host is fail-closed about who may dial it: its allowlist contains exactly one `peer_id` — the demo client's. The client, in turn, needs only the host's public key and the host's `peer_id` to trust the connection.

## 2. Run the host

You need Node.js ≥ 20 with `pnpm`. Clone the repository and install once:

```bash
git clone https://github.com/42ch-dev/spoke.git
cd spoke
pnpm install
```

The CLIs run from built output. Build the workspace packages the built CLIs import at runtime, plus the demo packages themselves (`examples/connect-demo/README.md`):

```bash
pnpm -F @42ch/spoke-schemas build        # compile-time prerequisite: generated wire types
pnpm -F @42ch/spoke-connect build        # runtime dep of both built demo CLIs
pnpm -F @42ch/spoke-operations build     # runtime dep of the built server CLI
pnpm -F @42ch/spoke-demo-server build
pnpm -F @42ch/spoke-demo-client build
```

In **terminal 1**, start the host:

```bash
node examples/connect-demo/server/dist/main.js --port 8787
```

It prints its identity, its allowlist, and the URL it listens on:

```text
SPOKE connect demo — mock inference host
  peer_id:   12D3KooWNm5t4HypYRmiC5v9CD2TnPKrJh2J8TcfJ2gPhA7L8TiZ
  allowlist: 12D3KooWM82bDYYgzgXaayHDdVciFe3bGvJ69qHnbSztNUJ933VQ
  listening: ws://127.0.0.1:8787
  (Ctrl+C to stop)
```

The `peer_id` is the host's trust root — derived from its Ed25519 public key, exactly as in the first tutorial. The printed allowlist is the demo client's `peer_id`: this host will accept a dial only from that peer. Leave the host running.

## 3. Implement `WsTransport`

A `Transport` is a consumer-implemented seam that carries connect envelopes between the adapter and the remote peer. It is **message-oriented**: one call moves exactly one connect envelope — `send(envelope)` sends one, `recv()` returns the next inbound one (rejecting when the connection closes), and `close()` releases resources and is idempotent. The full contract table is in [RemoteAdapter over a Transport](/how-to/connect-remote-adapter).

The demo client implements the seam over the `ws` WebSocket package (`examples/connect-demo/client/src/transport/ws-transport.ts`):

```ts
import { WebSocket } from "ws";

import type { EnvelopeBytes, Transport } from "@42ch/spoke-connect/remote";

/** A pending `recv` waiter. */
type RecvWaiter = {
  resolve: (bytes: EnvelopeBytes) => void;
  reject: (error: Error) => void;
};

/** View a `ws` message payload as envelope bytes (fresh per message). */
function toEnvelopeBytes(data: unknown): EnvelopeBytes {
  if (Buffer.isBuffer(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  return new Uint8Array(data as ArrayBuffer);
}
```

WebSocket already frames messages, so one WS message is exactly one connect envelope — no length-prefix delimiting needed on this carrier. The class keeps an inbound buffer for messages that arrive before anything calls `recv`, and a queue of pending `recv` waiters for the other direction:

```ts
export class WsTransport implements Transport {
  readonly #socket: WebSocket;
  /** Resolves once the socket is open; rejects if the connect fails. */
  readonly #open: Promise<void>;
  #closed = false;
  readonly #buffer: EnvelopeBytes[] = [];
  readonly #waiters: RecvWaiter[] = [];

  constructor(url: string) {
    this.#socket = new WebSocket(url);
    this.#open = new Promise<void>((resolve, reject) => {
      this.#socket.once("open", () => resolve());
      this.#socket.once("error", (error) => {
        reject(
          error instanceof Error
            ? error
            : new Error(`ws connect to ${url} failed`),
        );
      });
    });
    this.#socket.on("message", (data) => this.#push(toEnvelopeBytes(data)));
    // Both events fail pending recvs — a drop always surfaces as close/error.
    const fail = (): void => this.#failPending(new Error("ws connection closed"));
    this.#socket.on("close", fail);
    this.#socket.on("error", fail);
  }
```

`send` waits for the socket to open, then writes one envelope:

```ts
  async send(envelope: EnvelopeBytes): Promise<void> {
    await this.#open;
    if (this.#closed || this.#socket.readyState !== WebSocket.OPEN) {
      throw new Error("WsTransport is closed");
    }
    await new Promise<void>((resolve, reject) => {
      this.#socket.send(envelope, (error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve();
      });
    });
  }
```

`recv` serves buffered messages first, then waits; `close` is idempotent and fails any pending `recv` so the adapter's in-flight invokes fail fast instead of waiting out their timeouts:

```ts
  recv(): Promise<EnvelopeBytes> {
    if (this.#closed) {
      return Promise.reject(new Error("WsTransport is closed"));
    }
    const buffered = this.#buffer.shift();
    if (buffered !== undefined) {
      return Promise.resolve(buffered);
    }
    return new Promise<EnvelopeBytes>((resolve, reject) => {
      this.#waiters.push({ resolve, reject });
    });
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#failPending(new Error("WsTransport is closed"));
    this.#socket.close();
  }

  #push(bytes: EnvelopeBytes): void {
    const waiter = this.#waiters.shift();
    if (waiter !== undefined) {
      waiter.resolve(bytes);
      return;
    }
    this.#buffer.push(bytes);
  }

  #failPending(error: Error): void {
    for (const waiter of this.#waiters.splice(0)) {
      waiter.reject(error);
    }
  }
}
```

That is the whole seam. Every connect envelope the adapter sends or receives flows through these three methods; the adapter handles all session rules on top of them.

## 4. Dial with `connectRemoteAdapter`

With the transport in hand, dialing is one call. `connectRemoteAdapter` performs the signed-hello exchange, the allowlist check, and the session snapshot verification, then resolves to an established adapter (`examples/connect-demo/client/src/main.ts`):

```ts
export async function runDemoClient(options: {
  url: string;
}): Promise<DemoClientRun> {
  const transport = new WsTransport(options.url);
  const adapter = await connectRemoteAdapter({
    transport,
    localIdentity: { seed: DEMO_CLIENT_SEED },
    localManifest: DEMO_CLIENT_MANIFEST,
    remotePubkey: DEMO_SERVER_PUBKEY,
    allowlist: [DEMO_SERVER_PEER_ID],
  });
```

The options mirror the first tutorial's session concepts:

- `transport` — your `Transport` implementation; the adapter sends and receives envelopes through it.
- `localIdentity.seed` — your 32-byte Ed25519 seed; the adapter signs your hello with it.
- `localManifest` — your `HostCapabilityManifest`, advertised in the signed hello. The demo client is an `input-source` app in the same `demo-harbor` namespace:

```ts
export const DEMO_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: ["spoke-baseline"],
  namespaces: [DEMO_SCOPE_ID],
  extensions: {},
};
```

- `remotePubkey` — the host's 32-byte Ed25519 public key. The remote `peer_id` is derived from it, and it must be on the allowlist (fail-closed). The demo ships fixed identity seeds — DEMO ONLY; production apps must generate their own Ed25519 keys. The client keeps its own copy of the host's public key and `peer_id` (`examples/connect-demo/client/src/identities.ts`):

```ts
/** Public key derived from {@link DEMO_SERVER_SEED} — the remote key the client trusts. */
export const DEMO_SERVER_PUBKEY = getPublicKeyEd25519(DEMO_SERVER_SEED);

/** peer_id derived from {@link DEMO_SERVER_PUBKEY} — the client's allowlist entry. */
export const DEMO_SERVER_PEER_ID = derivePeerIdFromEd25519Pubkey(
  DEMO_SERVER_PUBKEY,
);
```

In a real integration, key distribution is transport-adapter-owned: you obtain the host's public key out of band and pin it, exactly as the demo pins its constants.

- `allowlist` — the peer ids this adapter accepts; the remote `peer_id` must be listed. A dial failure — wrong key, missing allowlist entry, handshake rejection — rejects the `connectRemoteAdapter` promise, and no adapter instance exists.

Run the client in **terminal 2**:

```bash
node examples/connect-demo/client/dist/main.js --url ws://127.0.0.1:8787
```

The dial establishes, and the CLI prints the session:

```text
SPOKE connect demo — third-party client
  dialing ws://127.0.0.1:8787 as 12D3KooWM82bDYYgzgXaayHDdVciFe3bGvJ69qHnbSztNUJ933VQ
  remote peer: 12D3KooWNm5t4HypYRmiC5v9CD2TnPKrJh2J8TcfJ2gPhA7L8TiZ (demo-inference-host)
    capabilities: spoke-baseline
    namespaces:   demo-harbor
```

The remote peer id matches the host's printed `peer_id`, and the manifest is the server manifest you met in section 1 — the adapter exposes it as `adapter.remoteManifest`, cached at session establish.

## 5. Call the ports

The adapter implements the async `BaselinePorts` six families, so you call knowledge, relation, scope, finding, rule, and host-manifest methods as if the peer were local. The demo client exercises the knowledge family with optimistic concurrency (`examples/connect-demo/client/src/main.ts`):

```ts
  // Step 1 — capability manifest (cached at establish, no round-trip).
  const serverManifest = adapter.remoteManifest;

  // Step 2 — put → get round-trip with OCC: create, then compare-and-swap.
  const created = requireOk(
    await adapter.putKnowledgeEntry(SUBMITTED_ENTRY, null),
  );
  if (created.revision === undefined) {
    throw new Error("demo client: created entry has no revision");
  }
  const updated = requireOk(
    await adapter.putKnowledgeEntry(
      { ...SUBMITTED_ENTRY, status: "confirmed" },
      created.revision,
    ),
  );
  const fetched = requireOk(
    await adapter.getKnowledgeEntry(SUBMITTED_ENTRY.entry_id),
  );

  // Step 3 — list: seed corpus + submitted entry + engine-derived artifacts.
  const listed = requireOk(
    await adapter.listKnowledgeEntries({ scope_id: DEMO_SCOPE_ID }),
  );

  // Step 4 — findings round-trip.
  const findings = requireOk(await adapter.putFindings([SUBMITTED_FINDING]));

  // Step 5 — peer host manifests (the demo host knows no peers).
  const peerManifests = requireOk(
    await adapter.listPeerHostCapabilityManifests(),
  );
```

Two things are worth noticing here.

First, `putKnowledgeEntry` is conditional: the second argument is the expected base revision. `null` means **create** — the entry must not exist yet; a number means **compare-and-swap** — the store's current revision must equal it. The first put creates the entry at revision 1; the second put passes `created.revision` and updates the entry to revision 2 (and flips its status to `confirmed`). Revisions are store-owned — the host assigns them, never the caller. The submitted entry is a plain `KnowledgeEntry`:

```ts
const SUBMITTED_ENTRY: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "demo-harbor/item/compass",
  entry_type: "item",
  canonical_name: "Compass",
  status: "provisional",
  body: { summary: "A brass compass." },
  extensions: {},
};
```

Second, every port call settles to a `SpokeResult` — a discriminated `{ ok: true, value }` / `{ ok: false, code, message }` union — instead of throwing. The demo unwraps with a small helper that fails loudly on rejection (`examples/connect-demo/client/src/main.ts`):

```ts
/** Unwrap a port-call result or fail the demo loudly (no silent fallbacks). */
function requireOk<T>(result: AnySpokeResult<T>): T {
  if (!result.ok) {
    throw new Error(
      `demo client: port call rejected (${result.code}): ${result.message}`,
    );
  }
  return result.value;
}
```

`getHostCapabilityManifest` is special: it is the session cache, served from the signed hello at establish — no round-trip. That is why the client reads `adapter.remoteManifest` instead of calling the port.

## 6. See the mock inference

The host's engine watches the store and derives its own artifacts after every accepted mutation. Look at the list output the CLI prints at the end of its run:

```text
  listKnowledgeEntries → 4 entries (demo-harbor/character/mira, demo-harbor/location/harbor, derived/world-digest, demo-harbor/item/compass)
```

The first two entries are the seed corpus; `demo-harbor/item/compass` is the entry you put; `derived/world-digest` is the engine's. Every accepted put re-runs the derivation, which builds a reserved-id KnowledgeEntry (`examples/connect-demo/server/src/engine/mock-engine.ts`):

```ts
    const digest: KnowledgeEntry = {
      schema_version: 1,
      entry_id: DERIVED_WORLD_DIGEST_ENTRY_ID,
      entry_type: "note",
      canonical_name: "World Digest",
      status: "confirmed",
      body: {
        summary: `Digest of ${userEntries.length} knowledge entries in demo-harbor.`,
        computable: {
          entry_type_counts: entryTypeCounts,
          entry_ids_sorted: sortedIds,
        },
      },
      revision: this.derivationCount,
      extensions: {},
    };
```

After the demo flow, the digest reads:

```json
{
  "schema_version": 1,
  "entry_id": "derived/world-digest",
  "entry_type": "note",
  "canonical_name": "World Digest",
  "status": "confirmed",
  "body": {
    "summary": "Digest of 3 knowledge entries in demo-harbor.",
    "computable": {
      "entry_type_counts": {
        "character": 1,
        "location": 1,
        "item": 1
      },
      "entry_ids_sorted": [
        "demo-harbor/character/mira",
        "demo-harbor/item/compass",
        "demo-harbor/location/harbor"
      ]
    }
  },
  "revision": 3,
  "extensions": {}
}
```

The digest's `revision` equals the derivation count — it advances on every accepted put, so the artifact is a stable function of user history. The `derived/` id namespace is reserved: user puts into it are rejected. This is what a real inference host's output looks like through the same `BaselinePorts` surface: derived knowledge appears in ordinary listings and reads, indistinguishable in shape from user data.

## 7. Handle errors

Two classes of failure matter, and they surface differently.

**Dial failures happen before an adapter exists.** If the host rejects your hello — wrong allowlist, wrong key, nonce replay — `connectRemoteAdapter` rejects and you have no adapter to close. The demo proves the allowlist path end to end: a third identity, `DEMO_STRANGER_SEED`, trusts the server but is not on the server's allowlist. The server-side allowlist check fails the hello and closes the socket mid-dial, so the client's dial fails fast with the connection loss (`examples/connect-demo/client/tests/e2e.test.ts`):

```ts
  it("rejects a dial from a non-allowlisted stranger identity", async () => {
    const transport = new WsTransport(server.url);
    transports.push(transport);

    // The stranger's OWN allowlist trusts the server, so the dial is
    // attempted; the SERVER-side allowlist rejects the hello and closes the
    // socket, failing the dial fast — no session is established. The
    // rejection is the handshake's connection loss (the server hung up
    // mid-dial), not a bare any-error assertion.
    await expect(
      connectRemoteAdapter({
        transport,
        localIdentity: { seed: DEMO_STRANGER_SEED },
        localManifest: DEMO_CLIENT_MANIFEST,
        remotePubkey: DEMO_SERVER_PUBKEY,
        allowlist: [DEMO_SERVER_PEER_ID],
      }),
    ).rejects.toThrow(/ws connection closed/);

    transport.close();
  });
```

**Port-call failures settle to `SpokeResult` rejects.** On an established session, every port call either resolves `ok` or rejects with `{ ok: false, code, message, details }` — your code branches on `result.ok` or unwraps with a helper like `requireOk`. Rejects carry the wire code (`REVISION_CONFLICT` for a create on an existing id, `STORED_REVISION_STALE` for a stale base revision, `INVALID_INPUT` for a reserved `derived/` id, …) and, for infrastructure failures, a `details.kind` that tells you which layer failed — including `transport` (I/O), `session_closed` (connection lost — stop the host and watch in-flight calls reject), and `timeout` (that call only; the session stays usable). The complete failure table lives in [RemoteAdapter over a Transport](/how-to/connect-remote-adapter).

That is the whole error surface: dial rejects before the adapter exists, port calls reject after it — two classes, and each surfaces through one channel.

## What you now know

- What a connect host looks like from the outside: a `BaselinePorts` adapter behind a signed-hello responder, advertising capabilities and namespaces in its manifest.
- The message-oriented `Transport` seam: one envelope per `send`/`recv`, `recv` rejects on close, `close` is idempotent — and a complete WebSocket implementation of it.
- How to dial with `connectRemoteAdapter`: `transport`, `localIdentity`, `localManifest`, `remotePubkey`, and a fail-closed `allowlist`.
- The `BaselinePorts` call pattern: conditional puts with optimistic concurrency, `SpokeResult` returns, and `getHostCapabilityManifest` served from the session cache.
- Where host-side inference shows up: derived artifacts with reserved ids appearing in ordinary listings.
- How failures surface: dial rejection before an adapter exists, `SpokeResult` rejects with `details.kind` after.

## Next steps

- [RemoteAdapter over a Transport](/how-to/connect-remote-adapter) — the task-oriented counterpart: the full option tables, concurrency rules, and the error mapping.
- [Route across multiple peers](/how-to/multi-peer-routing) — compose several established adapters behind one `BaselinePorts` surface.
- [Connect wire reference](/reference/connect) — envelope field tables, envelope authentication, and the port-method ops catalogue.
