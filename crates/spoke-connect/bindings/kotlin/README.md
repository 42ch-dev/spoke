# spoke-connect Kotlin binding

JVM library for SPOKE Connect session-core bindings (committed uniffi Kotlin + JNA native load).

Packaging contract: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.5.

## Install (Maven)

Coordinates: **`io.github.42ch-dev:spoke-connect`** on GitHub Packages (`https://maven.pkg.github.com/42ch-dev/spoke`).

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
    implementation("io.github.42ch-dev:spoke-connect:X.Y.Z")
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

## Layout

| Path | Contents |
|------|----------|
| `generated/uniffi/spoke_connect/` | Committed generated Kotlin (post-generate patch applied) |
| `bindgen/` | Post-generate patch recipe (`message` → `detail` on four `CoreException` variants) |
| `native/<jna-rid>/` | Committed host native for smoke; jar also packs CI-assembled `src/main/resources/` |
| `src/main/resources/<jna-rid>/` | CI-assembled JNA classpath natives (gitignored; see `assemble-kotlin-natives.sh`) |
| `Smoke/` | Golden-parity smoke (`gradle test`) |
| `build.gradle.kts` | JVM library, `maven-publish`, smoke harness |

JNA resource prefixes inside the jar: `darwin-aarch64/`, `linux-x86-64/`, `win32-x86-64/`.

## Maintainer: regenerate → patch → stage native → smoke / publish

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

Release publish (CI `publish-maven` on stable tags): assembles ffi-matrix natives via `./tooling/connect/assemble-kotlin-natives.sh`, then `gradle publish` to GitHub Packages with `GITHUB_TOKEN`.

Full patch rationale: [`bindgen/README.md`](bindgen/README.md).

## Kotlin / Rust field naming

Rust FFI error variants keep payload fields named `message` (wire-stable for published C# and other bindings). uniffi Kotlin codegen conflicts with `Throwable.message`; the committed binding applies a **post-generate rename** to `` `detail` `` in Kotlin sources only — no Rust FFI changes.
