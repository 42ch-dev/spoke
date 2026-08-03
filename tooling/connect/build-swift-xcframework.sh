#!/usr/bin/env bash
# Regenerate committed Swift bindings + macOS arm64 xcframework for SPM.
#
# Usage (from repo root):
#   ./tooling/connect/build-swift-xcframework.sh
#
# Outputs (committed paths):
#   crates/spoke-connect/bindings/swift/generated/
#   crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "${REPO_ROOT}"

SWIFT_BINDINGS="${REPO_ROOT}/crates/spoke-connect/bindings/swift"
GENERATED="${SWIFT_BINDINGS}/generated"
XCFRAMEWORK="${SWIFT_BINDINGS}/xcframework/spoke_connectFFI.xcframework"

# Prefer nightly locally (AGENTS.md); fall back to default cargo (CI stable).
CARGO=(cargo)
if command -v rustup >/dev/null 2>&1 && rustup toolchain list | grep -q '^nightly'; then
  CARGO=(cargo +nightly)
fi

TARGET_DIR="$("${CARGO[@]}" metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

echo "==> build ffi cdylib (bindgen metadata source)"
"${CARGO[@]}" build -p spoke-connect --features ffi --release
CDYLIB="${TARGET_DIR}/release/libspoke_connect.dylib"
if [[ ! -f "${CDYLIB}" ]]; then
  echo "missing cdylib: ${CDYLIB}" >&2
  exit 1
fi

echo "==> generate Swift bindings"
mkdir -p "${GENERATED}"
"${CARGO[@]}" run -p spoke-connect --features bindgen-cli --bin uniffi-bindgen -- \
  generate --library "${CDYLIB}" \
  --language swift \
  --out-dir "${GENERATED}"

echo "==> build staticlib slice (macOS arm64 host)"
"${CARGO[@]}" rustc -p spoke-connect --features ffi --release --crate-type staticlib
STATICLIB="${TARGET_DIR}/release/libspoke_connect.a"
if [[ ! -f "${STATICLIB}" ]]; then
  echo "missing staticlib: ${STATICLIB}" >&2
  exit 1
fi

echo "==> stage xcframework headers (module.modulemap + FFI header only)"
HDRS="$(mktemp -d)"
trap 'rm -rf "${HDRS}"' EXIT
cp "${GENERATED}/spoke_connectFFI.h" "${HDRS}/"
cat > "${HDRS}/module.modulemap" <<'EOF'
module spoke_connectFFI {
    header "spoke_connectFFI.h"
    export *
}
EOF

echo "==> create xcframework (macOS arm64 first slice)"
rm -rf "${XCFRAMEWORK}"
mkdir -p "$(dirname "${XCFRAMEWORK}")"
xcodebuild -create-xcframework \
  -library "${STATICLIB}" \
  -headers "${HDRS}" \
  -output "${XCFRAMEWORK}"

echo "==> done"
echo "  generated: ${GENERATED}"
echo "  xcframework: ${XCFRAMEWORK}"
echo "Validate: swift build (repo root) and Smoke/README.md swiftc smoke"
