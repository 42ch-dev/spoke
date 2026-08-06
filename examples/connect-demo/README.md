# Connect demo — mock inference host + third-party RemoteAdapter client

A runnable two-package TypeScript demo of the **connect wire family over a real WebSocket**:

- `server/` (`@42ch/spoke-demo-server`) — a deterministic **mock inference host**: a `BaselinePorts` adapter backed by a pure rule-based engine, served by a spec-faithful connect responder (`ConnectHost`) over a `ws` WebSocketServer, with an allowlist + signed-hello + per-envelope auth (`protocol_version` 2).
- `client/` (`@42ch/spoke-demo-client`) — a **third-party-style client**: its own `Transport` implementation over `ws`, then the real library client (`connectRemoteAdapter` from `@42ch/spoke-connect/remote`) dials the host and calls the drop-in async `BaselinePorts` surface.

The client never touches session-core verification helpers — exactly what a third-party integrator would do against a SPOKE host.

## Run it (two terminals)

Prerequisites: `pnpm install` once. The CLIs run from built output, so build the demo packages first (this also builds nothing else — the workspace deps' dist is only needed at CLI runtime, see below):

```bash
pnpm -F @42ch/spoke-connect build   # runtime dep of the built demo (tests never need it)
pnpm -F @42ch/spoke-demo-server build
pnpm -F @42ch/spoke-demo-client build
```

**Terminal 1 — the host:**

```bash
node examples/connect-demo/server/dist/main.js --port 8787
```

prints the host's `peer_id`, the allowlist (only the demo client's `peer_id`), and the listening URL, then waits for dials.

**Terminal 2 — the third-party client:**

```bash
node examples/connect-demo/client/dist/main.js --url ws://127.0.0.1:8787
```

dials the host and prints each story step: remote manifest (session cache), `putKnowledgeEntry` create + compare-and-swap, `getKnowledgeEntry`, `listKnowledgeEntries` (seed corpus + submitted entry + engine-derived `derived/world-digest`), `putFindings`, and `listPeerHostCapabilityManifests → []`.

One command runs the whole flow as a gate:

```bash
pnpm -F @42ch/spoke-demo-client test        # e2e: boots the host on an ephemeral port
pnpm -F @42ch/spoke-demo-server test        # engine/adapter/responder unit + loopback suites
pnpm ci:typescript                          # full repo gate, including both demo packages
```

## File → concept map

| File | Concept it teaches |
|------|--------------------|
| `server/src/engine/mock-engine.ts`, `seed-corpus.ts` | Deterministic inference: rule-based derivation over an in-memory store with a fixed seed corpus (no LLM, no randomness). |
| `server/src/adapter/mock-adapter.ts` | The `BaselinePorts` adapter a host serves: knowledge/relation/scope/finding/rule/host-manifest families with OCC. |
| `server/src/identities.ts` | Fixed demo Ed25519 identities (server, client, and a non-allowlisted stranger) — `DEMO ONLY`, never reuse. |
| `server/src/host/envelope-auth.ts` | Spec-derived `protocol_version` 2 envelope authentication: the three algorithm ids over the locked field sets, with `canonicalize` (RFC 8785). |
| `server/src/host/connect-host.ts` | The responder: allowlist → hello verify → nonce → signed responder hello + session snapshot, then per-invoke gate (sequence peek → auth verify → advance) + dispatch. |
| `server/src/host/port-dispatch.ts` | D4 port-op catalogue: `port.*` op → adapter method mapping + capability requirements. |
| `server/src/transport/ws-server.ts` | The server end of the D3 transport seam + `serveConnectDemo({ port })` (port 0 = ephemeral, used by the e2e). |
| `server/src/main.ts` | Server CLI: `--port`, prints peer id + allowlist + URL. |
| `client/src/transport/ws-transport.ts` | The consumer `Transport` contract: one connect envelope per `send`/`recv`, `recv` rejects on close, idempotent `close`. |
| `client/src/main.ts` | The third-party story: `connectRemoteAdapter` + `BaselinePorts` only — `runDemoClient` returns the asserted results, the CLI prints them. |
| `client/src/identities.ts` | The client's own copy of the demo identities (it must not import the server package at runtime). |
| `client/tests/e2e.test.ts` | The end-to-end gate: real WebSocket, full flow, negative allowlist proof, process hygiene. |

## Dependency surface

The third-party story is that a client needs **only two SPOKE packages** plus a WebSocket library:

- `@42ch/spoke-demo-client` runtime deps: `@42ch/spoke-connect` + `@42ch/spoke-schemas` + `ws`.
- The demo server adds `@42ch/spoke-operations` + `canonicalize` (RFC 8785, pinned to the exact version the library pins so signatures are byte-identical).

The demo server is a **devDependency** of the client (used only by the e2e to boot the host). Neither package is published (`"private": true`), and neither imports into `@42ch/spoke-connect` internals — the demo implements envelope auth per the normative field sets with the public crypto helpers.

## Docs

The RemoteAdapter how-to walks through the same contract the demo exercises — the `Transport` seam, the dial, and the `BaselinePorts` calls: [docs/how-to/connect-remote-adapter.md](../../docs/how-to/connect-remote-adapter.md). A step-by-step integration tutorial that uses this demo as its runnable spine is planned on the [roadmap](../../.mstar/roadmap.md) ("RemoteAdapter integration tutorial (EN+CN)").
