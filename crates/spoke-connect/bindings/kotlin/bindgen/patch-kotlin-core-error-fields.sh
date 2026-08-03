#!/usr/bin/env bash
# Post-generate patch for uniffi Kotlin: CoreException payload fields named
# `message` conflict with kotlin.Throwable.message. Rust FFI keeps `message`
# (wire-stable for C#/Swift/Python); Kotlin generated sources rename the payload
# property to `detail` after every stock generate.
#
# Usage (from repo root):
#   ./crates/spoke-connect/bindings/kotlin/bindgen/patch-kotlin-core-error-fields.sh \
#     crates/spoke-connect/bindings/kotlin/generated/uniffi/spoke_connect/spoke_connect.kt

set -euo pipefail

TARGET="${1:-crates/spoke-connect/bindings/kotlin/generated/uniffi/spoke_connect/spoke_connect.kt}"

if [[ ! -f "${TARGET}" ]]; then
  echo "missing generated Kotlin: ${TARGET}" >&2
  exit 1
fi

# Property declarations on InvalidNonce, Crypto, Jcs, TokenInvalid
perl -pi -e '
  s/val `message`: kotlin\.String/val `detail`: kotlin.String/g;
  s/get\(\) = "message=\$\{ `message` \}"/get() = "detail=\${ `detail` }"/g;
  s/value\.`message`/value.`detail`/g;
' "${TARGET}"

echo "patched CoreException payload fields (message -> detail): ${TARGET}"
