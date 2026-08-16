---
module: spoke-connect
date: 2026-08-06
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when:
  - "exposing an async Rust library (receive loop, per-invoke timeouts) over a sync FFI facade for foreign bindings"
  - "deciding who owns the tokio runtime when a cdylib crosses a sync/async boundary"
  - "bridging a blocking foreign-callback interface (sync send/recv/close) into an async trait seam"
  - "propagating close and fail-fast-on-close semantics across an FFI boundary"
  - "gating test-only binding fixtures (loopback smoke hosts) out of the production cdylib"
  - "mapping an async SpokeResult reject vocabulary to a foreign-language error enum"
  - "extending a sync FFI surface with a responder/accept side (host-owned listen/accept, factory with no error path)"
  - "serving host logic through a foreign callback (tool handler) over the same sync block-on-async bridge"
  - "running per-language binding smokes in each channel's default suite against committed artifacts"
last_updated: 2026-08-17
tags:
  - spoke-connect
  - ffi-boundary
  - uniffi
  - block-on-async
  - tokio
  - foreign-callback
  - remote-adapter
  - feature-gating
---

# Sync block-on-async FFI bridge over an async Rust library

## Context

The connect stack exposes its session rules through a deliberately **sync, core-only** FFI facade (`crates/spoke-connect/src/ffi.rs`): pure functions and small objects, no runtime, no I/O — see [`connect-session-core-ffi-boundary.md`](connect-session-core-ffi-boundary.md). The `RemoteAdapter` (an encapsulated async client: dial + six `BaselinePorts` families + read-only session info + `close`) lives above that core, behind the `remote-adapter` feature. It is genuinely **async**: it owns a continuous receive loop (`tokio::spawn(receive_loop)` calling `transport.recv().await`), demultiplexes responses by `request_id`, and runs per-invoke timeout timers (`tokio::spawn(sleep(timeout))`). Its `Transport` seam is async (`send` / `recv` / `close`, `Send + Sync`), and `recv` **fails fast on close** (`TransportError::Closed`) so in-flight invokes are rejected instead of hanging until timeout.

Native bindings (Swift, Kotlin, Python, C#/Go) cannot call async Rust directly. The bridge pattern that shipped exposes the async adapter as a **synchronous object** — `RemoteAdapterFFI` — whose methods each `block_on` the async adapter over a cdylib-owned tokio runtime, with the binding supplying `Transport` as a foreign-callback interface and a feature-gated loopback smoke host for golden-parity binding tests.

This doc records the seven load-bearing decisions of that bridge as durable guidance for any future async-over-FFI surface in this repo or elsewhere, plus the surface-extension rules proven when the surface grew a responder/accept side and foreign tool-serving faces (§8). The long-term facts are here and in `.mstar/specs/spoke-remote-adapter.md`.

## Guidance

### 1. The cdylib owns the tokio runtime — one process-wide `current_multi_thread` instance, behind a non-default feature

The FFI facade owns the runtime; the foreign host never does. A crate-private `OnceLock<Runtime>`, lazily built on the first FFI call, never re-created, shared by every FFI entry point:

```rust
#[cfg(feature = "ffi")]
static FFI_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub(crate) fn ffi_runtime() -> &'static tokio::runtime::Runtime {
    FFI_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("cdylib tokio runtime initializes once")
    })
}
```

**`current_multi_thread` is required, not a preference.** The adapter's receive loop is a long-running task that must keep making progress regardless of which (or whether any) foreign thread is inside `block_on`. On a `current_thread` runtime, tasks only advance while a `block_on` is being polled on that exact thread; a foreign OS thread's `block_on` cannot drive the receive loop, and concurrent foreign `block_on` calls would serialize on one thread. Multi-thread parks the receive loop on a worker and guarantees per-invoke timer tasks fire while a foreign thread is blocked awaiting its response. The runtime feature-gates the whole surface: `ffi = ["dep:uniffi", "tokio/rt-multi-thread"]`, so default builds stay lean and runtime-free.

Rules: one runtime per process, shared by all FFI objects (a router composing adapters reuses the same `OnceLock` instance — never a second runtime); `Handle::block_on` is callable from a foreign, non-async OS thread.

### 2. Sync block-on-async surface — each FFI method `block_on`s the async method

Every port method wraps the async call in `ffi_runtime().block_on(...)` and maps the result. The foreign host sees a synchronous object; the runtime does the waiting:

```rust
pub fn get_knowledge_entry(&self, entry_id: String) -> Result<String, FfiError> {
    map_spoke_result(
        ffi_runtime().block_on(self.inner.get_knowledge_entry(&entry_id)),
    )
}
```

Structured values cross as JSON strings (deserialized with `serde_json` inside Rust — no generated schema types on the FFI surface); keys cross as `Vec<u8>` validated to 32 bytes.

Concurrency contract, stated explicitly in the docs: **one in-flight invoke per calling OS thread**. Concurrent invokes require concurrent caller threads; the runtime demultiplexes responses by `request_id`. The outbound send lock is held only through allocate → sign → wire-send and released before awaiting the response, so concurrent invokes reach the transport in allocation order but settle in parallel. True async FFI (callback/poll completion handles, streaming over FFI) is a deliberate non-goal; revisit it only if a binding needs streaming or concurrent invokes over a single FFI thread.

### 3. Foreign-callback `Transport` bridge — each blocking callback runs via `spawn_blocking`, never on an async worker

The binding implements `Transport` in its own language as a **synchronous** foreign-callback interface (uniffi `callback_interface`): `send` accepts one envelope's bytes, `recv` blocks until an envelope arrives or the connection closes, `close` is idempotent resource release:

```rust
#[uniffi::export(callback_interface)]
pub trait Transport: Send + Sync {
    fn send(&self, envelope: Vec<u8>) -> Result<(), TransportError>;
    fn recv(&self) -> Result<Vec<u8>, TransportError>;
    fn close(&self) -> Result<(), TransportError>;
}
```

A `ForeignCallbackTransport` bridges this sync interface to the async `Transport` seam: each callback call is executed through the shared runtime's `spawn_blocking` pool:

```rust
#[async_trait]
impl transport::Transport for ForeignCallbackTransport {
    async fn recv(&self) -> Result<Vec<u8>, transport::TransportError> {
        let inner = Arc::clone(&self.inner);
        ffi_runtime()
            .spawn_blocking(move || inner.recv())
            .await
            .map_err(|join| {
                transport::TransportError::Io(format!("recv task failed: {join}"))
            })?
            .map_err(Into::into)
    }
    // send / close identical
}
```

Because a binding's `recv` blocks the calling OS thread, running it directly on an async worker (or via `block_in_place`) would monopolize that worker. `spawn_blocking` is the canonical tokio bridge: the blocking call runs on the runtime's **blocking thread pool**, the adapter's receive loop stays a normal async task, and only the foreign-callback invocation is offloaded. The error enums map 1:1 in both directions (`Closed` ↔ `Closed`, `Io` ↔ `Io`).

In-repo, the loopback transport pair (client + server ends of one in-memory connection) is re-exported over FFI as objects whose methods each `block_on` the async loopback — the same sync block-on-async surface a binding uses, and the fixture for every binding smoke.

### 4. Close-fail-fast propagation — the async close chain crosses the boundary unchanged

Close ordering is fixed end-to-end and is **the same code path the async adapter already uses**; the FFI layer only adds the `spawn_blocking` bridge and the per-call `block_on`:

1. Foreign `close` callback is invoked.
2. The in-flight `recv` bridge returns `Err(TransportError::Closed)` — a foreign-callback `Transport` MUST mirror the loopback contract: `close` makes a pending or next `recv` return `Closed` (buffered messages are lost, matching a real connection drop).
3. The receive loop's `transport.recv().await` returns the `Err`; the loop calls `close_session("transport loss: …")`.
4. `close_session` marks the adapter state `Closed`, aborts every pending invoke's timeout task, and settles each waiter with `RemoteErrorKind::SessionClosed`.
5. Each blocked foreign thread's `block_on` resolves with that reject; the FFI surface maps it to `session_closed`.

No new close handling is invented at the FFI layer. Close is idempotent (`close_session` early-returns when already `Closed`). Fail-fast-on-close exists so a connection loss rejects in-flight invokes immediately instead of making every waiter sit out its invoke timeout.

### 5. Binding smoke-host feature gating — test-only fixtures behind a dedicated non-default feature

The in-repo loopback smoke host (fixed test seeds, reference fixture adapter, parametric variant for multi-peer routing smokes) is gated behind `ffi-smoke-host` — a feature that is **not implied by `ffi` or `remote-adapter`**:

```toml
remote-adapter = ["dep:spoke-operations"]
ffi-smoke-host = ["remote-adapter", "dep:spoke-fixture-toy-world"]
```

```rust
#[cfg(feature = "ffi-smoke-host")]
pub fn start_loopback_smoke_host(server: Arc<LoopbackTransport>) -> Arc<LoopbackSmokeHost> { … }
```

The production cdylib — the surface foreign bindings generate from — stays free of test fixtures; binding smokes regenerate with the feature enabled and exercise the same seeds/manifests the Rust parity tests use. Rule: any fixture whose only consumers are binding smokes gets its own feature; never widen the shipped surface for test convenience.

### 6. Error mapping — one FFI error enum mirroring the async error vocabulary, faithful passthrough

The FFI error surface is a two-variant uniffi enum. The split cleanly separates **constructor failures** (thrown before any adapter exists) from **invoke-path rejects**:

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("dial failed ({kind}): {message}")]
    Dial { kind: String, message: String },
    #[error("rejected ({code}): {message}")]
    Rejected {
        code: String,
        message: String,
        kind: Option<String>,
        wire_code: Option<String>,
    },
}
```

- `Dial { kind: "config" | "handshake" | "timeout" }` — dial/constructor failures (`RemoteAdapterError`) before an adapter exists.
- `Rejected { code, message, kind?, wire_code? }` — invoke-path `SpokeResult::Reject` passthrough that carries the full frozen-contract D7 vocabulary: application reject codes preserved verbatim; `INTERNAL_ERROR` rows carry `kind` (`transport`, `session_closed`, `timeout`, `correlation_mismatch`, `sequence_exhausted`, `envelope_auth_missing` / `envelope_auth_invalid` / `envelope_auth_session_unbound`); dispatch deny (`op_unsupported` / `capability_missing`) and unknown wire codes carry `wire_code`.

The mapping is a **faithful passthrough**: it does not invent, merge, or drop any error class from the async vocabulary. Unit tests pin each mapping (dial kinds, all eight internal kinds verbatim, application rejects without kind/wire_code, dispatch deny, unknown wire codes) so a future vocabulary change cannot silently alter the FFI surface.

### 7. Panic containment — `catch_unwind` at every exported `block_on` site, mapped to the existing error slot

A panicking future must never unwind across the FFI boundary (UB territory for foreign callers). Every exported FFI-object `block_on` site routes through a shared `ffi_block_on` / `ffi_block_on_void` / `ffi_block_on_transport` helper that wraps the call in `catch_unwind(AssertUnwindSafe(...))`:

- **Result-returning sites** map a caught panic to the **existing** `FfiError::Rejected { code: "INTERNAL_ERROR", kind: Some("panic"), wire_code: None }` — the D7 `kind` slot already carries internal-error subtypes, so containment adds **no new error variant** (the frozen surface is unchanged; D7's table simply gains a `panic` row).
- **`()`-returning sites** (`close`) have no error slot: catch + log + swallow by design — a close-time panic cannot be surfaced, and hiding it cannot mask invoke-path errors.
- **Start-with-Result variants** (the smoke host's `start_loopback_smoke_host_variant`) propagate the mapped error via `?`; the no-error-slot sibling keeps an explicitly documented re-panic that preserves the original message — the single sanctioned exception, recorded in code and in D7.
- **Test injection stays test-only**: the panic-injection flag is `thread_local!` + `#[cfg(test)]`, thread-keyed so concurrent tests cannot consume each other's injection, and compiles to zero in release cdylibs — production has no wrapper state machine beyond `catch_unwind` itself (the `#[cfg(test)]` split keeps it that way).
- uniffi 0.32's own scaffolding independently wraps sync exports in a `rust_call` `catch_unwind`, so containment is double-layered: the explicit helper gives the mapped `FfiError`; the scaffolding is the backstop for anything missed.

`AssertUnwindSafe` is sound here because each future is created fresh per call and dropped (never resumed) after a caught panic; the shared runtime is unaffected.

### 8. Extending the surface — responder + tool faces

The same bridge composes when the FFI surface grows an accept side and foreign-callback tool serving (`invoke_tool` on the adapter / router / responder, `register_tool_handler`, `connect_responder_ffi`). The rules below are the durable extensions proven by those faces; they refine the six decisions above, not replace them.

#### 8.1 Factory semantics mirror the library honestly — never fake block-on-handshake

When the library factory has no error path and returns a `Handshaking` handle immediately (the responder's run loop owns the handshake; the dialer's hello is the sync point), the FFI constructor returns immediately in `Handshaking` too — `block_on` covers only the factory future, which completes instantly. Handshake failure — allowlist rejection, hello-verify rejection, or a peer that never appears — surfaces as `state() → "Closed"` plus transport close, with no timeout at this layer: a never-appearing peer holds `Handshaking` until the transport closes. The `Result` slot is reserved for FFI-side config validation only: manifest JSON, seed length, peer-key length → `FfiError::Dial { kind: "config" }`. A constructor draft that invented "block on until handshake completes, `Dial { kind: "handshake" | "timeout" }` on failure" was not implementable — the library has no handshake awaitable and no handshake-timeout parameter — and was rejected. A factory with no error path must not be given one at the FFI layer; consumers observe the handshake outcome through bounded `state()` polling, the same contract the library and its TS consumers use.

#### 8.2 Accept topology — the host owns listen/accept; FFI constructors take an already-connected callback `Transport`

FFI never builds a listener. The host product process holds listen/accept in its own language's network stack, wraps the connected socket in its callback `Transport`, and passes it to `connect_responder_ffi` — the exact mirror of dial, where `connect_remote_adapter_ffi` takes a connected outbound `Transport`. Listener policy (bind, TLS, backlog, accept timeouts) stays with the host; the FFI surface adds no accept-callback face and no new concept. The loopback proof drives both ends of `loopback_transport_pair()` — dial FFI on one end, responder FFI on the other — and smokes poll `state()` to `Established` before invoking.

#### 8.3 Foreign callback for host logic (ToolHandler pattern) — `spawn_blocking` + a strict outcome map

Tool serving crosses the boundary as a synchronous foreign callback — `ToolHandler::handle(arguments_json) -> Result<String, FfiError>` — bridged exactly like `Transport` (§3): each call runs on the shared runtime's `spawn_blocking` pool, never on an async worker, and the `JoinError` from a panicking or abandoned handler is the foreign-crash signal. The outcome map is the contract:

- `Ok(json)` → success value, parsed inside the bridge; malformed success JSON is contained, never trusted.
- `Err(FfiError::Rejected { .. })` → application reject passthrough: `code` and `message` preserved, `kind` / `wire_code` re-hung onto the reject's `details` — the inverse of the invoke-path mapping (§6).
- `Err(FfiError::Dial { .. })` (outside the handler contract), any other foreign exception, and the blocking-task `JoinError` → `INTERNAL_ERROR` containment with `details: None`, mirroring the in-library `catch_unwind` (§7); the serve loop survives.
- Unknown reject code strings downgrade to `INTERNAL_ERROR` with the message preserved — the typed wire enum cannot represent them, and inventing a row would widen the frozen vocabulary.

#### 8.4 Grammar pre-validation lives in the FFI wrapper, before the library call

Foreign input never reaches a library programmer-error panic. `register_tool_handler` pre-validates `capability_id` with the library's public `parse_tool_capability_id` inside the FFI wrapper before calling the library, so the library's grammar panic is unreachable across the boundary. Invalid grammar rejects `FfiError::Rejected { code: "INVALID_INPUT", kind: None, wire_code: None }` with the offending id in `message` and zero wire traffic — the same row `invoke_tool` returns for the same input. The FFI register face returns `Result<(), FfiError>`, never a panic; valid ids register last-wins on the library face and never mutate the manifest.

#### 8.5 Re-entrancy caveat

Foreign handlers run on the blocking pool — one pool thread per in-flight call. A handler must not synchronously call back into FFI faces from inside `handle` (a handler that calls `invoke_tool` on the same adapter parks a pool thread or deadlocks); hosts hand off asynchronously instead. The caveat is recorded in every binding doc, next to the handler contract.

#### 8.6 Blocking-pool sizing

Every established FFI session pins one blocking-pool thread per transport end (each end's `recv` blocks inside `spawn_blocking`), so a full-duplex session consumes two pool threads. At the tokio default blocking-pool cap that is roughly 256 full-duplex sessions per process. Binding docs record the sizing note so hosts planning large fan-out account for the pool.

#### 8.7 Smoke-gating convention — feature smokes run in each channel's DEFAULT suite against committed production artifacts

Per-language feature smokes must run in the channel's default suite, against committed production artifacts (committed natives / xcframework). A smoke bundled behind a dev-only build gate — a smoke-host tag/define/class (for example `-p:SmokeHost=true` → `SMOKE_HOST` in C#) — silently proves nothing on shipped artifacts: the default consumer build never runs it. This was caught at plan QC: two of five languages proved the tool faces by default before the fix; all five did after. Test-only fixtures keep their own non-default feature (§5); the smoke that proves the shipped surface stays in the default path.

#### 8.8 Positive control before the five-channel regen

After changing the FFI surface, run `uniffi-bindgen generate --library` against the cdylib plus a parse/compile check for one first-party binding BEFORE dispatching the five-channel regeneration — a cheap signal that the surface shape is valid before it multiplies across channels. Vendored fork channels (C#/Go) need an additional per-feature check for new uniffi shapes the community generator may lag on; this round's instance was the fielded error enum (`FfiError`) as a callback throws type, cleared before regen.

## Why This Matters

An FFI boundary is a compatibility contract: once a foreign language imports the surface, every change to it is a breaking change for that language. This pattern keeps the contract small and correct by deciding the four hardest questions up front:

- **Runtime ownership.** If the foreign host owned the runtime, every binding would need a tokio-equivalent lifecycle; if the facade created a runtime per call, stateful async tasks (the receive loop) could not survive across calls. A process-wide cdylib-owned runtime gives bindings a plain synchronous object with no runtime dependency of their own.
- **Blocking inside async.** A blocking foreign `recv` run on an async worker stalls the worker and, with it, the receive loop — a close or timeout could then never be processed (deadlock). `spawn_blocking` offloads only the callback invocation and keeps the async machinery alive.
- **Close semantics.** Without fail-fast-on-close across the boundary, every waiter hangs until its invoke timeout on connection loss — silent latency and no clear session-closed signal. The chain (foreign close → `TransportError::Closed` → loop exit → waiters reject) reuses the async adapter's existing behavior instead of inventing FFI-specific close handling.
- **Surface hygiene.** Test fixtures leaking into the production cdylib widen the exported surface for no consumer benefit; the dedicated feature keeps smokes reproducible without polluting the shipped contract. And an error enum that drops or merges D7 classes would break binding parity with the async contract the surface mirrors.

## When to Apply

- Exposing an async Rust library — long-running receive loop, per-invoke timeouts, `Send + Sync` traits — over a sync FFI facade for foreign bindings that cannot call async Rust.
- Choosing who owns the runtime at a sync/async boundary: if the library owns long-lived async tasks, the cdylib (not the host) must own the runtime.
- Bridging a blocking foreign-callback interface (sync `send` / `recv` / `close`) into an async trait seam: always through `spawn_blocking`, never on an async worker.
- Any FFI surface over a transport with close semantics: propagate fail-fast-on-close unchanged, and require the foreign callback to mirror the loopback contract (`close` fails pending/next `recv`).
- Shipping golden-parity binding smokes with in-repo fixtures: gate the fixtures behind a dedicated non-default feature that the production cdylib does not enable.
- Mapping an async reject vocabulary (`SpokeResult::Reject` with codes/kinds/wire codes) to a foreign error enum: mirror it 1:1, split constructor failures from invoke rejects, and pin the mapping with unit tests.
- Extending the surface with a responder/accept side: host owns listen/accept; the FFI constructor takes an already-connected callback `Transport`; a factory with no error path stays `Handshaking` at the FFI layer with handshake outcome observed via `state()`.
- Serving host logic through a foreign callback (tool handler): bridge with `spawn_blocking`, map outcomes strictly (`Ok` → success, `Rejected` → passthrough, everything else → `INTERNAL_ERROR` containment), pre-validate foreign input in the FFI wrapper, and never call back into FFI faces from inside a handler.
- Running per-language feature smokes in each channel's default suite against committed production artifacts, with test-only fixtures behind their own non-default feature; after an FFI surface change, run a one-binding positive control before regenerating all channels.

## Examples

The shipped surface in `crates/spoke-connect/src/ffi.rs` composes all six decisions. The constructor shows runtime ownership, the callback bridge, and dial-error mapping in one place:

```rust
#[uniffi::export]
pub fn connect_remote_adapter_ffi(
    transport: Box<dyn FfiTransport>,
    local_seed: Vec<u8>,
    local_manifest_json: String,
    remote_pubkey: Vec<u8>,
    allowlist: Vec<String>,
    invoke_timeout_ms: Option<u64>,
) -> Result<Arc<RemoteAdapterFFI>, FfiError> {
    let adapter = ffi_runtime()
        .block_on(connect_remote_adapter(RemoteAdapterOptions {
            transport: Arc::new(ForeignCallbackTransport::new(Arc::from(transport))),
            local_identity: RemoteIdentity { seed: ed25519_seed(local_seed)? },
            local_manifest: serde_json::from_str(&local_manifest_json).map_err(|error| {
                FfiError::Dial { kind: "config".into(), message: format!("invalid local host manifest JSON: {error}") }
            })?,
            remote_pubkey: ed25519_pubkey(remote_pubkey)?,
            allowlist,
            invoke_timeout_ms,
            capability_token: None,
        }))
        .map_err(map_dial_error)?;
    Ok(Arc::new(RemoteAdapterFFI { inner: adapter }))
}
```

The composition rule for the close chain (point 4) is: the binding's `close` callback → the in-flight `spawn_blocking` `recv` returns `Closed` → the receive loop's `transport.recv().await` errors → `close_session("transport loss: …")` marks `Closed` and rejects every pending waiter → each blocked foreign `block_on` resolves with the mapped `session_closed`. A multi-peer router composed over these adapters reuses the same runtime and error enum — no second runtime, no second error surface.

## See also

- [`connect-session-core-ffi-boundary.md`](connect-session-core-ffi-boundary.md) — the sync core-only FFI facade this bridge sits **above** (pure session rules; the 8 functions + 3 objects + 2 enums remain unchanged — the async bridge is purely additive).
- [`encapsulated-remote-adapter-bridge.md`](encapsulated-remote-adapter-bridge.md) — the async RemoteAdapter bridge pattern (operations ports ↔ connect session, message-oriented `Transport` seam, per-peer session state) that this FFI layer wraps.
- [`connect-golden-vector-ssot.md`](connect-golden-vector-ssot.md) — the cross-language parity methodology that proves the sync FFI surface produces outcomes identical to the async adapter (loopback oracle).
- [`connect-uniffi-bindgen-fork.md`](connect-uniffi-bindgen-fork.md) — community bindgen verification when a binding generator lags the pinned uniffi line.
- [`reverse-invoke-on-symmetric-wire.md`](reverse-invoke-on-symmetric-wire.md) — the request/response wire semantics the tool faces ride (request-shape-first classification, auth-before-advance serving order, self-describing `tools.*` capability model).
- [`connect-error-taxonomy-layering.md`](connect-error-taxonomy-layering.md) — the four-layer error chain; the `FfiError` layer that §8's outcome map and error rows extend by reference.
