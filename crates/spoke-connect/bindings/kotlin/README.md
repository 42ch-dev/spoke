# spoke-connect Kotlin binding

JVM library for SPOKE Connect session-core bindings (committed uniffi Kotlin + JNA native load).

Packaging contract: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.5.

Maven coordinates (Task 4+): `io.github.42ch-dev:spoke-connect` on GitHub Packages (`maven.pkg.github.com/42ch-dev/spoke`).

## Usage

```kotlin
import uniffi.spoke_connect.derivePeerIdFromEd25519Pubkey
import uniffi.spoke_connect.protocolVersion

val peerId = derivePeerIdFromEd25519Pubkey(pubkey)
val version = protocolVersion() // 1
```

Transport / WebSocket stays in the host product.

## Layout

| Path | Contents |
|------|----------|
| `generated/uniffi/spoke_connect/` | Committed generated Kotlin (post-generate patch applied) |
| `bindgen/` | Post-generate patch recipe (`message` → `detail` on four `CoreException` variants) |
| `native/<rid>/` | Committed cdylib per platform for smoke / JNA (see `native/README.md`) |
| `Smoke/` | Golden-parity smoke (`gradle test`) |
| `build.gradle.kts` | JVM library + smoke test harness |

## Maintainer: regenerate → patch → stage native → smoke

Commands from the **repository root** (local nightly convention: `cargo +nightly …`):

```bash
cargo +nightly build -p spoke-connect --features ffi
cargo +nightly run -p spoke-connect --features bindgen-cli --bin uniffi-bindgen -- \
  generate --library target/debug/libspoke_connect.dylib \
  --language kotlin \
  --out-dir crates/spoke-connect/bindings/kotlin/generated \
  --no-format
./crates/spoke-connect/bindings/kotlin/bindgen/patch-kotlin-core-error-fields.sh
mkdir -p crates/spoke-connect/bindings/kotlin/native/darwin-aarch64
cp target/debug/libspoke_connect.dylib crates/spoke-connect/bindings/kotlin/native/darwin-aarch64/
install_name_tool -id @rpath/libspoke_connect.dylib \
  crates/spoke-connect/bindings/kotlin/native/darwin-aarch64/libspoke_connect.dylib
cd crates/spoke-connect/bindings/kotlin && gradle test
```

Full patch rationale: [`bindgen/README.md`](bindgen/README.md).

## Kotlin / Rust field naming

Rust FFI error variants keep payload fields named `message` (wire-stable for published C# and other bindings). uniffi Kotlin codegen conflicts with `Throwable.message`; the committed binding applies a **post-generate rename** to `` `detail` `` in Kotlin sources only — no Rust FFI changes.
