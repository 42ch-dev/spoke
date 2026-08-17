# spoke-connect Kotlin binding

JVM library for SPOKE Connect session-core bindings (committed uniffi Kotlin + JNA native load).

Packaging contract: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.5.

## Install (Maven)

Coordinates: **`dev.42ch:spoke-connect`** on GitHub Packages (`https://maven.pkg.github.com/42ch-dev/spoke`).

At spoke lockstep tag `vX.Y.Z`:

```kotlin
// settings.gradle.kts or build.gradle.kts
repositories {
    maven {
        url = uri("https://maven.pkg.github.com/42ch-dev/spoke")
        credentials {
            username = providers.gradleProperty("gpr.user").get()
            password = providers.gradleProperty("gpr.key").get()
        }
    }
}

dependencies {
    implementation("dev.42ch:spoke-connect:X.Y.Z")
    // JNA is a transitive dependency of the published artifact
}
```

```kotlin
import uniffi.spoke_connect.derivePeerIdFromEd25519Pubkey
import uniffi.spoke_connect.protocolVersion

val peerId = derivePeerIdFromEd25519Pubkey(pubkey)
val version = protocolVersion() // 1
```

Namespace: **`uniffi.spoke_connect`**. Transport / WebSocket stays in the host product.

## Build features

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed `generated/` + `native/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `MultiPeerRouterFFI`, `Transport`, `loopbackTransportPair`, the tool faces (`invokeTool`, `registerToolHandler`, `ToolHandler`, `connectResponderFfi` / `ConnectResponderFfi`) — **no** `startLoopbackSmokeHost` |
| Local loopback smoke cdylib + bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.
Full loopback smoke procedure: [`Smoke/README.md`](Smoke/README.md).

## RemoteAdapter FFI surface

With `remote-adapter` enabled, the binding ships the additive remote-adapter surface: `RemoteAdapterFFI` (single peer), `MultiPeerRouterFFI` (multi-peer routing), the callback `Transport` interface, the in-memory loopback helpers, and the tool faces — `invokeTool` on the adapter, router, and responder, `registerToolHandler` on the adapter and responder, the `ToolHandler` callback, and `connectResponderFfi` / `ConnectResponderFfi` for the accept side.

### Transport contract

The callback `Transport` is a message-oriented interface: one envelope per call, blocking receive, idempotent close.

| Method | Behavior |
|--------|----------|
| `send(envelope)` | Accepts exactly one connect envelope's bytes per call |
| `recv()` | Blocks until the next inbound envelope arrives or the connection closes; returns exactly one envelope per call |
| `close()` | Releases transport resources; idempotent — closing either end of a connection fails the peer's pending `recv` like a real connection drop |

The surface bounds messages at one envelope per call; byte-stream carriers apply length-prefix (or equivalent) delimiting before handing envelopes to the adapter.

### MultiPeerRouterFFI

`newMultiPeerRouterFfi()` returns the router as a synchronous object over the same runtime: a peer registry (`registerPeer(adapter)` accepts an established `RemoteAdapterFfi` and returns its `peer_id`; `unregisterPeer(peerId)`; `listPeers()`), the `BaselinePorts` six families routed per call to exactly one capable peer, and the two `HostManifestPort` aggregation views — the composed `getHostCapabilityManifest()` and the per-peer `listPeerHostCapabilityManifests()`. Selection matches each registered peer's cached `HostCapabilityManifest`: required capability, exact namespace, soft role preference, and a deterministic lowest-`peer_id` tie-break.

### Tool faces

- `invokeTool(capabilityId, argumentsJson)` — invoke a tool on the peer (dialer → responder, responder → dialer, or router → capable peer); returns the tool's `result` as a JSON string. A non-`tools.` id or malformed arguments rejects `INVALID_INPUT` with zero wire traffic; a dispatch deny rejects `CAPABILITY_PORT_MISSING` with the peer's preserved `wireCode`.
- `registerToolHandler(capabilityId, handler)` — serve reverse invokes through a foreign `ToolHandler`; last-wins per id, never mutates the manifest. The callback's `handle(argumentsJson)` returns the result JSON; a thrown `FfiException.Rejected` passes through verbatim as an application reject, any other outcome is contained to `INTERNAL_ERROR` and the session survives.
- `connectResponderFfi(...)` / `ConnectResponderFfi` — the accept side: wrap a connected (host-accepted) callback `Transport`. The constructor returns immediately in `Handshaking` — poll `state()` (bounded) to `Established` before invoking; a handshake failure surfaces as `state() == "Closed"` (never a thrown constructor error), config-validation failures return `FfiException.Dial` with `kind == "config"`.
- Handlers run on the FFI blocking pool — do not synchronously call back into the FFI faces from `handle`; hand off asynchronously in the host instead.

## Layout

| Path | Contents |
|------|----------|
| `generated/uniffi/spoke_connect/` | Committed generated Kotlin (post-generate patch applied) |
| `bindgen/` | Post-generate patch recipe — see `bindgen/README.md` (`CoreException`/`FfiException` `message`→`detail`, domain `close()` collision on loopback types) |
| `native/<jna-rid>/` | Committed host native for smoke; jar also packs CI-assembled `src/main/resources/` |
| `src/main/resources/<jna-rid>/` | CI-assembled JNA classpath natives (gitignored; see `assemble-kotlin-natives.sh`) |
| `Smoke/` | Golden-parity smoke (`gradle test`) |
| `loopback-smoke/` | RemoteAdapter loopback smoke (requires `-PsmokeHost=true`; see `Smoke/README.md`) |
| `build.gradle.kts` | JVM library, `maven-publish`, smoke harness |

JNA resource prefixes inside the jar: `darwin-aarch64/`, `linux-x86-64/`, `win32-x86-64/`.

## Maintainer: regenerate → patch → stage native → smoke / publish

Commands from the **repository root** (local nightly convention: `cargo +nightly …`):

```bash
cargo +nightly build -p spoke-connect --features ffi,remote-adapter --release
cargo +nightly run -p spoke-connect --features ffi,bindgen-cli --bin uniffi-bindgen -- \
  generate --library target/release/libspoke_connect.dylib \
  --language kotlin \
  --out-dir crates/spoke-connect/bindings/kotlin/generated \
  --no-format
./crates/spoke-connect/bindings/kotlin/bindgen/patch-kotlin-core-error-fields.sh
mkdir -p crates/spoke-connect/bindings/kotlin/native/darwin-aarch64
cp target/release/libspoke_connect.dylib crates/spoke-connect/bindings/kotlin/native/darwin-aarch64/
install_name_tool -id @rpath/libspoke_connect.dylib \
  crates/spoke-connect/bindings/kotlin/native/darwin-aarch64/libspoke_connect.dylib
cd crates/spoke-connect/bindings/kotlin && gradle test
```

Release publish (CI `publish-maven` on stable tags): assembles ffi-matrix natives via `./tooling/connect/assemble-kotlin-natives.sh`, then `gradle publish` to GitHub Packages with `GITHUB_TOKEN`.

Full patch rationale: [`bindgen/README.md`](bindgen/README.md).

## Kotlin / Rust field naming

Rust FFI error variants keep payload fields named `message` (wire-stable for published C# and other bindings). uniffi Kotlin codegen conflicts with `Throwable.message`; the committed binding applies a **post-generate rename** to `` `detail` `` in Kotlin sources only — no Rust FFI changes.
