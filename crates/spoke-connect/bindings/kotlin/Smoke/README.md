# Kotlin golden-parity smoke

JVM smoke for the uniffi Kotlin bindings of the `spoke-connect` sync-core facade.
Asserts golden peer id, hello signature, verify round-trip, tamper rejection, and
protocol version `1`.

## Run

From `crates/spoke-connect/bindings/kotlin/` (requires host native under
`native/<rid>/` — see `native/README.md`):

```bash
gradle test
```

From the repository root after staging `native/darwin-aarch64/libspoke_connect.dylib`:

```bash
cd crates/spoke-connect/bindings/kotlin && gradle test
```

Override native path explicitly:

```bash
gradle test -PnativeLib="$PWD/native/darwin-aarch64/libspoke_connect.dylib"
```

Expected: 5 tests PASS (`GoldenParityTest`).

Regenerate sequence: [`../bindgen/README.md`](../bindgen/README.md).
