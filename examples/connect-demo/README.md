# Connect demo — mock inference host + third-party client with reverse tool invocation

A runnable two-package TypeScript demo of the **connect wire family over a real WebSocket**, including the reverse-tool surface:

- `server/` (`@42ch/spoke-demo-server`) — a deterministic **mock inference host**: a `BaselinePorts` adapter backed by a pure rule-based engine, served by the library responder (`connectResponder` from `@42ch/spoke-connect/remote`) over a `ws` WebSocketServer. The host **discovers the dialer's tools from the authenticated manifest** and **reverse-invokes one mid-orchestration**, feeding the result into a `BaselinePorts` step. It also serves the whole-operation `ke-extraction` service over a host-local loader + extractor, and answers viewpoint-bearing scope queries (`ke-ownership`) against its seeded governance fixtures.
- `client/` (`@42ch/spoke-demo-client`) — a **third-party-style client**: its own `Transport` implementation over `ws`, then the real library client (`connectRemoteAdapter` from `@42ch/spoke-connect/remote`) dials the host, **registers two deterministic toy-world tools** on its `RemoteAdapter` (`tools.toy_world.roll_dice` and `tools.toy_world.lore_lookup` — the same frozen ids as the reference provider in `fixtures/toy-world/`), and calls the drop-in async `BaselinePorts` surface.

The client never touches session-core verification helpers — exactly what a third-party integrator would do against a SPOKE host.

## The story, in one walkthrough

1. **Dial.** The client opens a real WebSocket to the host and establishes an authenticated connect session (`connectRemoteAdapter`). Its manifest advertises two tools — `tools.toy_world.roll_dice` (deterministic dice: same arguments, same rolls) and `tools.toy_world.lore_lookup` (read-only lore lookup over the client's own store) — plus the `toy_world` namespace it owns.
2. **Discover.** The host reads the client's manifest from the authenticated session (`remoteManifest`), validates it (`validateManifestTools`), and lists its `tools[]` — both toy-world tools.
3. **Reverse-invoke mid-orchestration.** When the client submits its knowledge entry (a compass in `demo-harbor`), the host runs an orchestration step: it asks the client to roll 2d6 by reverse-invoking `tools.toy_world.roll_dice` with `{ count: 2, sides: 6 }` — a normal signed connect invoke in the reverse direction. The client's registered handler runs on the client and answers with `{ rolls, total }`.
4. **Feed the result into a BaselinePorts step.** The host records the roll as a knowledge entry (`demo-harbor/artifact/dice-roll`) in its engine — a `BaselinePorts` orchestration step. The client's next `listKnowledgeEntries` shows the dice-roll artifact carrying the exact roll result.
5. **List through a viewpoint — the ownership witness.** The listing runs with `{ scope_id: "demo-harbor", viewpoint: "demo-harbor/character/mira" }`, so the host's disclosure predicate decides visibility: the shared fixture `demo-harbor/note/harbor-log` and the viewpoint holder's own `demo-harbor/note/mira-private-log` come back — owner, `disclosure` and unknown extension namespaces verbatim — while the foreign holder's `demo-harbor/note/rival-private-log` is withheld. The rest of the listing is unaffected: the submitted entry, the derived `derived/world-digest` and the orchestration's `demo-harbor/artifact/dice-roll` stay visible. The client declares `ke-ownership`, the supplementary capability a viewpoint-bearing scope query needs.
6. **Ask the host to extract.** The client sends a reference-only `ExtractRequest` (the `extract` core op, capability `ke-extraction`): source anchors plus a `run_id`, never source content. The host loads those sources in its own process and answers one provisional candidate per referenced source (`demo-harbor/extracted/<run_id>/<n>`) correlated by that `run_id` — the loaded value never crosses the wire.
7. **Deny, not silent success.** A client that does not list (and so does not negotiate) a tool gets a capability deny for a reverse invoke: the wire answers `op_unsupported`, which the library maps to `CAPABILITY_PORT_MISSING`. The host records the deny — it never pretends the tool call succeeded.

Because `roll_dice` is seeded from its arguments, the roll for `{ count: 2, sides: 6 }` is always `{ rolls: [1, 2], total: 3 }` — the e2e asserts this exact value.

## Disclosure boundary

The demo's **only** disclosure enforcement is the knowledge scope-listing predicate: `listKnowledgeEntries` filters through the library `filterKnowledgeEntriesByScope`, whose `viewpoint` conjunct is the core `knowledgeEntryVisibleToViewpoint` rule — shared entries (no `disclosure`) plus the viewpoint holder's own `owner-private` entries, never a foreign holder's. Governance travels back verbatim (owner, disclosure, unknown extension namespaces). The two timeline queries (`listTimelineEvents`, `listForkTimelineEvents`) go through `filterTimelineEventsByScope`, which refines by declared ids / scale / fork only and carries no viewpoint conjunct at all.

Every other read path in the demo is **disclosure-unaware**: `getKnowledgeEntry` answers an entry by id with no owner or disclosure check, and the derived `derived/world-digest` is computed over the whole corpus with the derived ids excluded — the private fixtures included — so it is not viewpoint-filtered and its body lists every user entry id. The demo therefore demonstrates the predicate, not a disclosure-complete host; a real host owns whatever enforcement its disclosure model requires beyond the scope query.

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

dials the host, registers both toy-world tools on its `RemoteAdapter`, and prints each story step: remote manifest (session cache), the registered tools, `putKnowledgeEntry` create + compare-and-swap (during which the host reverse-invokes `roll_dice`), `getKnowledgeEntry`, `listKnowledgeEntries` under the viewpoint-bearing scope (seed world pair + submitted entry + engine-derived `derived/world-digest` + the orchestration's `demo-harbor/artifact/dice-roll`, with the foreign holder's private entry withheld), `putFindings`, `listPeerHostCapabilityManifests → []`, `extract` (the host's provisional candidates for the requested sources), and the optional families — `project` → `compute` (settle) and `listForkTimelineEvents` over the seeded storm fork.

One command runs the whole flow as a gate:

```bash
pnpm -F @42ch/spoke-demo-client test        # e2e: boots the host on an ephemeral port
pnpm -F @42ch/spoke-demo-server test        # engine/adapter/orchestration unit + loopback suites
pnpm ci:typescript                          # full repo gate, including both demo packages
```

## File → concept map

| File | Concept it teaches |
|------|--------------------|
| `server/src/engine/mock-engine.ts`, `seed-corpus.ts` | Deterministic inference: rule-based derivation over an in-memory store with a fixed seed corpus (no LLM, no randomness). The corpus is the world pair plus **three governance fixtures** — shared `demo-harbor/note/harbor-log`, own-private `demo-harbor/note/mira-private-log` (owner `demo-harbor/character/mira`) and foreign-private `demo-harbor/note/rival-private-log` (owner `demo-harbor/character/rival`) — which make the ownership predicate observable; the engine derives `derived/world-digest` over the user corpus, derived ids excluded. |
| `server/src/adapter/mock-adapter.ts` | The `BaselinePorts` adapter a host serves: knowledge/relation/scope/finding/rule/host-manifest families with OCC, the viewpoint-bearing scope query through the library predicate, and the whole-operation `ke-extraction` service — `extract` runs the library `orchestrateExtract` over a host-local deterministic `ExtractionPort` (loader + extractor), never a second extraction path. The server manifest declares the same tool ids the client serves (so they negotiate), plus `ke-extraction` and `ke-ownership`. |
| `server/src/tools/toy-world-tools.ts` | The frozen tool ids + descriptors the demo negotiates (byte-parity with the reference provider). |
| `server/src/host/orchestration.ts` | The host's tool-assisted orchestration step: discovery from the authenticated manifest → reverse invoke mid-flow → feed the roll result into the engine; every run is recorded (discovery, result, deny path). The same object forwards the whole-operation `extract` call to the wrapped adapter instead of owning an extraction path. |
| `server/src/transport/ws-server.ts` | The server end of the D3 transport seam + `serveConnectDemo({ port })` (port 0 = ephemeral, used by the e2e). Each connection serves the library `connectResponder` with a `DemoOrchestrator` — the library responder is the demo's only serving path. |
| `server/src/main.ts` | Server CLI: `--port`, prints peer id + allowlist + URL. |
| `server/src/identities.ts` | Fixed demo Ed25519 identities (server, client, and a non-allowlisted stranger) — `DEMO ONLY`, never reuse. |
| `client/src/transport/ws-transport.ts` | The consumer `Transport` contract: one connect envelope per `send`/`recv`, `recv` rejects on close, idempotent `close`. |
| `client/src/tools/toy-world-tools.ts` | The client's copyable toy-world tool handlers (`roll_dice` + `lore_lookup`) — the deterministic algorithms the host reverse-invokes. |
| `client/src/main.ts` | The third-party story: `connectRemoteAdapter` + `registerToolHandler` + `BaselinePorts` only — `runDemoClient` returns the asserted results, the CLI prints them. |
| `client/src/identities.ts` | The client's own copy of the demo identities (it must not import the server package at runtime). |
| `client/tests/e2e.test.ts` | The end-to-end gate: real WebSocket, discovery → reverse invoke → result feeds orchestration, the negative capability-deny path, and the allowlist proof. |
| `server/tests/orchestration.test.ts` | The orchestration step over the loopback pair: discovery + reverse invoke + feed + deny, server-side. |

## Rust connect host bridge recipe

The connect wire family is not TypeScript-only: a Rust host serves the same `extract` service through the connect-owned `RemoteExtractService` face (`remote-adapter` feature) and drives the `spoke-operations` orchestration itself (`orchestrate_extract(&dyn ExtractionPort, request, extractor)`). The two ends disagree on one property, and the host must bridge it deliberately:

- the connect service face is `Send + Sync` (the responder dispatches it from its own task), while
- `orchestrate_extract` takes a plain `&dyn ExtractionPort` and therefore returns a `!Send` future.

**Recipe — own the host, clone the runtime handle, build the `!Send` future on a blocking worker** (no `Send`/`Sync` supertrait on `ExtractionPort`, no operations runtime dependency, no unsafe). Condensed from the tested host — the loader impl, the service's error branch and the full candidate assembly are elided:

```rust
impl RemoteExtractService for MyHost {
    async fn extract(&self, request: ExtractRequest) -> SpokeResult<ExtractResponse> {
        // 1. Own a `Send + Sync + 'static` clone of the host (it is the
        //    `ExtractionPort`) and a clone of the active runtime handle.
        let host = self.clone();
        let handle = tokio::runtime::Handle::current(); // multi-thread runtime
        // 2. Hand the whole call to a blocking worker and build + drive the
        //    `!Send` orchestration future INSIDE that closure.
        let joined = tokio::task::spawn_blocking(move || {
            handle.block_on(async move {
                orchestrate_extract(&host, request, move |input: ExtractRunInput| async move {
                    // The host's own extractor: local work over the loaded
                    // value, returning the provisional candidates.
                    spoke_ok(ExtractionResult {
                        candidates: vec![provisional_candidate(&input.request.run_id, 0)],
                        method: Some("canonical".to_owned()),
                        coverage_hint: None,
                    })
                })
                .await
            })
        })
        .await;
        match joined {
            Ok(result) => result,
            // 3. A panicked or cancelled worker surfaces a `JoinError`; answer
            //    the existing `INTERNAL_ERROR` row (no new wire code).
            Err(error) => spoke_reject(
                SpokeRejectCode::InternalError,
                format!("extract host task failed: {error}"),
                None,
            ),
        }
    }
}
```

The host implements `ExtractionPort` for its loader (a real host-local load, never a wire parameter) and attaches the service face with `RemoteServePortsComposite::with_extract(Arc::new(host))`; `ConnectResponder` then serves `extract` through gate → probe → decode → one call → signed response.

**Ownership.** The blocking closure owns the host clone, the request and the orchestration future; the `!Send` future is created, awaited and dropped inside the worker, so it never crosses a thread boundary and the async side only ever holds `Send` values. `Handle::block_on` enters the runtime context for the worker, which is what makes the host's own async stages usable there: they can yield, park on Tokio timers, and reach `tokio::runtime::Handle::current()` — use a **multi-thread** runtime so the timer/IO drivers keep running while the worker blocks.

**`JoinError` mapping.** A worker that panics (or is cancelled) fails the join; the service face answers the existing application reject `INTERNAL_ERROR` with the worker's failure in `message` — never a silent success and never a panic across the service boundary. No new wire vocabulary is introduced for it.

**The responder's serve timeout stops waiting — it does not terminate the worker.** The responder's local serve budget (`invokeTimeoutMs` / `invoke_timeout_ms`, default 5000 ms) bounds the *wait* for this service call: on expiry it answers one signed `INTERNAL_ERROR` (`details.kind = "timeout"`) response and stops awaiting the call, while an already-started blocking worker keeps running to completion and its result is discarded (no second response, no rollback of host work). Neither side promises preemption or side-effect rollback — a host owns cooperative cancellation and its own limits on blocking work.

**This recipe is tested, not just documented.** `CanonicalExtractHost` in [`crates/spoke-connect/tests/remote_loopback.rs`](../../crates/spoke-connect/tests/remote_loopback.rs) is exactly this host — a host-local loader, an in-process extractor and the operations `orchestrate_extract` — and the `extract_send_bridge_` witnesses drive it from `tokio::spawn` on a multi-thread runtime with an async loader and extractor that yield and park on Tokio timers, asserting the run id / provisional result, the worker's thread ownership, and the `JoinError` → `INTERNAL_ERROR` mapping (from the repository root):

```bash
cargo +nightly test -p spoke-connect --features remote-adapter --test remote_loopback extract_send_bridge_ -- --nocapture
```

## Dependency surface

The third-party story is that a client needs **only two SPOKE packages** plus a WebSocket library:

- `@42ch/spoke-demo-client` runtime deps: `@42ch/spoke-connect` + `@42ch/spoke-schemas` + `ws`.
- The demo server adds `@42ch/spoke-operations` (the `BaselinePorts` surface + manifest-tools validation helpers).

The demo server is a **devDependency** of the client (used only by the e2e to boot the host). Neither package is published (`"private": true`). The server serves connections with the library's `connectResponder` — the library responder is what every connection runs.

## Docs

The RemoteAdapter how-to walks through the same contract the demo exercises — the `Transport` seam, the dial, tool registration, and the `BaselinePorts` calls: [docs/how-to/connect-remote-adapter.md](../../docs/how-to/connect-remote-adapter.md). A step-by-step integration tutorial uses this demo as its runnable spine, in English and 简体中文: [docs/tutorials/integrate-remote-adapter.md](../../docs/tutorials/integrate-remote-adapter.md) and its [CN twin](../../docs/zh/tutorials/integrate-remote-adapter.md).
