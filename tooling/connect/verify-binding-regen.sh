#!/usr/bin/env bash
# Verify committed Swift/Python uniffi bindings match fresh bindgen output (no drift).
#
# Usage (from repo root):
#   ./tooling/connect/verify-binding-regen.sh
#
# C# and Go are intentionally excluded: they require vendored uniffi-bindgen forks
# under crates/spoke-connect/bindings/{csharp,go}/bindgen/ (see connect-uniffi-bindgen-fork.md).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "${REPO_ROOT}"

if command -v rustup >/dev/null 2>&1; then
  if ! rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
    echo "==> install nightly toolchain (binding regen)"
    rustup toolchain install nightly --profile minimal
  fi
fi

CARGO=(cargo +nightly)

FFI_FEATURES="ffi,remote-adapter"
BINDGEN_FEATURES="ffi,bindgen-cli,remote-adapter"
STAGING="/tmp/binding-drift-check"
SWIFT_OUT="${STAGING}/swift"
PYTHON_OUT="${STAGING}/python"

COMMITTED_SWIFT="${REPO_ROOT}/crates/spoke-connect/bindings/swift/generated"
COMMITTED_PYTHON="${REPO_ROOT}/crates/spoke-connect/bindings/python/spoke_connect/__init__.py"

rm -rf "${STAGING}"
mkdir -p "${SWIFT_OUT}" "${PYTHON_OUT}"

echo "==> build production cdylib (${FFI_FEATURES})"
"${CARGO[@]}" build -p spoke-connect --features "${FFI_FEATURES}"

TARGET_DIR="$("${CARGO[@]}" metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
CDYLIB="${TARGET_DIR}/debug/libspoke_connect.so"
if [[ ! -f "${CDYLIB}" ]]; then
  CDYLIB="${TARGET_DIR}/debug/libspoke_connect.dylib"
fi
if [[ ! -f "${CDYLIB}" ]]; then
  CDYLIB="${TARGET_DIR}/debug/spoke_connect.dll"
fi
if [[ ! -f "${CDYLIB}" ]]; then
  echo "error: cdylib not found under ${TARGET_DIR}/debug/" >&2
  exit 1
fi

echo "==> regenerate Swift bindings"
"${CARGO[@]}" run -p spoke-connect --features "${BINDGEN_FEATURES}" --bin uniffi-bindgen -- \
  generate --library "${CDYLIB}" --language swift --out-dir "${SWIFT_OUT}"

echo "==> regenerate Python bindings"
"${CARGO[@]}" run -p spoke-connect --features "${BINDGEN_FEATURES}" --bin uniffi-bindgen -- \
  generate --library "${CDYLIB}" --language python --out-dir "${PYTHON_OUT}"

REGEN_PYTHON="${PYTHON_OUT}/spoke_connect.py"
if [[ ! -f "${REGEN_PYTHON}" ]]; then
  echo "error: expected ${REGEN_PYTHON} from python bindgen" >&2
  exit 1
fi

DRIFT=0

echo "==> diff Swift generated/"
if ! diff -ru "${SWIFT_OUT}" "${COMMITTED_SWIFT}"; then
  echo "::error::Committed Swift bindings differ from fresh uniffi-bindgen output. Regenerate per tooling/connect/build-swift-xcframework.sh." >&2
  DRIFT=1
fi

echo "==> diff Python __init__.py"
if ! diff -u "${REGEN_PYTHON}" "${COMMITTED_PYTHON}"; then
  echo "::error::Committed Python bindings differ from fresh uniffi-bindgen output. Regenerate per crates/spoke-connect/bindings/python/README.md." >&2
  DRIFT=1
fi

if [[ "${DRIFT}" -ne 0 ]]; then
  exit 1
fi

echo "==> binding regeneration OK (no drift)"
