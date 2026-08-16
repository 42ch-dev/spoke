# Connect demo — mock inference host + third-party client with reverse tool invocation

A runnable two-package TypeScript demo of the **connect wire family over a real WebSocket**, including the reverse-tool surface:

- `server/` (`@42ch/spoke-demo-server`) — a deterministic **mock inference host**: a `BaselinePorts` adapter backed by a pure rule-based engine, served by the library responder (`connectResponder` from `@42ch/spoke-connect/remote`) over a `ws` WebSocketServer. The host **discovers the dialer's tools from the authenticated manifest** and **reverse-invokes one mid-orchestration**, feeding the result into a `BaselinePorts` step.
- `client/` (`@42ch/spoke-demo-client`) — a **third-party-style client**: its own `Transport` implementation over `ws`, then the real library client (`connectRemoteAdapter` from `@42ch/spoke-connect/remote`) dials the host, **registers two deterministic toy-world tools** on its `RemoteAdapter` (`tools.toy_world.roll_dice` and `tools.toy_world.lore_lookup` — the same frozen ids as the reference provider in `fixtures/toy-world/`), and calls the drop-in async `BaselinePorts` surface.

The client never touches session-core verification helpers — exactly what a third-party integrator would do against a SPOKE host.

## The story, in one walkthrough

1. **Dial.** The client opens a real WebSocket to the host and establishes an authenticated connect session (`connectRemoteAdapter`). Its manifest advertises two tools — `tools.toy_world.roll_dice` (deterministic dice: same arguments, same rolls) and `tools.toy_world.lore_lookup` (read-only lore lookup over the client's own store) — plus the `toy_world` namespace it owns.
2. **Discover.** The host reads the client's manifest from the authenticated session (`remoteManifest`), validates it (`validateManifestTools`), and lists its `tools[]` — both toy-world tools.
3. **Reverse-invoke mid-orchestration.** When the client submits its knowledge entry (a compass in `demo-harbor`), the host runs an orchestration step: it asks the client to roll 2d6 by reverse-invoking `tools.toy_world.roll_dice` with `{ count: 2, sides: 6 }` — a normal signed connect invoke in the reverse direction. The client's registered handler runs on the client and answers with `{ rolls, total }`.
4. **Feed the result into a BaselinePorts step.** The host records the roll as a knowledge entry (`demo-harbor/artifact/dice-roll`) in its engine — a `BaselinePorts` orchestration step. The client's next `listKnowledgeEntries` shows the dice-roll artifact carrying the exact roll result.
5. **Deny, not silent success.** A client that does not list (and so does not negotiate) a tool gets a capability deny for a reverse invoke: the wire answers `op_unsupported`, which the library maps to `CAPABILITY_PORT_MISSING`. The host records the deny — it never pretends the tool call succeeded.

Because `roll_dice` is seeded from its arguments, the roll for `{ count: 2, sides: 6 }` is always `{ rolls: [1, 2], total: 3 }` — the e2e asserts this exact value.

## Run it (two terminals)

Prerequisites: `pnpm install` once. The CLIs run from built output, so build the workspace packages the built CLIs import at runtime plus the demo packages themselves. This is the complete build set — `@42ch/spoke-schemas` builds first as the compile-time prerequisite (fresh checkouts must build it explicitly: it has no `prepare` script and its `dist/` is gitignored, so the demo builds resolve its generated wire types through the package `types` field only after this step), `@42ch/spoke-connect` / `@42ch/spoke-operations` are runtime deps of the built demo CLIs, and the demo packages build last (tests never need any of this):

```bash
pnpm -F @42ch/spoke-schemas build        # compile-time prerequisite: generated wire types
pnpm -F @42ch/spoke-connect build        # runtime dep of both built demo CLIs (tests never need it)
pnpm -F @42ch/spoke-operations build     # runtime dep of the built server CLI (tests never need it)
pnpm -F @42ch/spoke-demo-server build
pnpm -F @42ch/spoke-demo-client build
```

**Terminal 1 — the host:**

```bash
node examples/connect-demo/server/dist/main.js --port 8787
```

prints the host's `peer_id`, the allowlist (only the demo client's `peer_id`), the listening URL, and a note that the host reverse-invokes `tools.toy_world.roll_dice` mid-orchestration, then waits for dials.

**Terminal 2 — the third-party client:**

```bash
node examples/connect-demo/client/dist/main.js --url ws://127.0.0.1:8787
```

dials the host, registers both toy-world tools on its `RemoteAdapter`, and prints each story step: remote manifest (session cache), the registered tools, `putKnowledgeEntry` create + compare-and-swap (during which the host reverse-invokes `roll_dice`), `getKnowledgeEntry`, `listKnowledgeEntries` (seed corpus + submitted entry + engine-derived `derived/world-digest` + the orchestration's `demo-harbor/artifact/dice-roll`), `putFindings`, and `listPeerHostCapabilityManifests → []`.

One command runs the whole flow as a gate:

```bash
pnpm -F @42ch/spoke-demo-client test        # e2e: boots the host on an ephemeral port
pnpm -F @42ch/spoke-demo-server test        # engine/adapter/orchestration unit + loopback suites
pnpm ci:typescript                          # full repo gate, including both demo packages
```

## File → concept map

| File | Concept it teaches |
|------|--------------------|
| `server/src/engine/mock-engine.ts`, `seed-corpus.ts` | Deterministic inference: rule-based derivation over an in-memory store with a fixed seed corpus (no LLM, no randomness). |
| `server/src/adapter/mock-adapter.ts` | The `BaselinePorts` adapter a host serves: knowledge/relation/scope/finding/rule/host-manifest families with OCC. The server manifest declares the same tool ids the client serves, so they negotiate. |
| `server/src/tools/toy-world-tools.ts` | The frozen tool ids + descriptors the demo negotiates (byte-parity with the reference provider). |
| `server/src/host/orchestration.ts` | The host's tool-assisted orchestration step: discovery from the authenticated manifest → reverse invoke mid-flow → feed the roll result into the engine; every run is recorded (discovery, result, deny path). |
| `server/src/transport/ws-server.ts` | The server end of the D3 transport seam + `serveConnectDemo({ port })` (port 0 = ephemeral, used by the e2e). Each connection serves the library `connectResponder` with a `DemoOrchestrator` — the dogfooded responder, no hand-rolled copy. |
| `server/src/main.ts` | Server CLI: `--port`, prints peer id + allowlist + URL. |
| `server/src/identities.ts` | Fixed demo Ed25519 identities (server, client, and a non-allowlisted stranger) — `DEMO ONLY`, never reuse. |
| `client/src/transport/ws-transport.ts` | The consumer `Transport` contract: one connect envelope per `send`/`recv`, `recv` rejects on close, idempotent `close`. |
| `client/src/tools/toy-world-tools.ts` | The client's copyable toy-world tool handlers (`roll_dice` + `lore_lookup`) — the deterministic algorithms the host reverse-invokes. |
| `client/src/main.ts` | The third-party story: `connectRemoteAdapter` + `registerToolHandler` + `BaselinePorts` only — `runDemoClient` returns the asserted results, the CLI prints them. |
| `client/src/identities.ts` | The client's own copy of the demo identities (it must not import the server package at runtime). |
| `client/tests/e2e.test.ts` | The end-to-end gate: real WebSocket, discovery → reverse invoke → result feeds orchestration, the negative capability-deny path, and the allowlist proof. |
| `server/tests/orchestration.test.ts` | The orchestration step over the loopback pair: discovery + reverse invoke + feed + deny, server-side. |

## Dependency surface

The third-party story is that a client needs **only two SPOKE packages** plus a WebSocket library:

- `@42ch/spoke-demo-client` runtime deps: `@42ch/spoke-connect` + `@42ch/spoke-schemas` + `ws`.
- The demo server adds `@42ch/spoke-operations` (the `BaselinePorts` surface + manifest-tools validation helpers).

The demo server is a **devDependency** of the client (used only by the e2e to boot the host). Neither package is published (`"private": true`). The server dogfoods the library's `connectResponder` — no hand-rolled responder path remains in the demo.

## Docs

The RemoteAdapter how-to walks through the same contract the demo exercises — the `Transport` seam, the dial, tool registration, and the `BaselinePorts` calls: [docs/how-to/connect-remote-adapter.md](../../docs/how-to/connect-remote-adapter.md). A step-by-step integration tutorial uses this demo as its runnable spine, in English and 简体中文: [docs/tutorials/integrate-remote-adapter.md](../../docs/tutorials/integrate-remote-adapter.md) and its [CN twin](../../docs/zh/tutorials/integrate-remote-adapter.md).
