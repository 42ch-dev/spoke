#!/usr/bin/env bash
# Regenerate committed Swift bindings + multi-slice xcframework for SPM.
#
# Usage (from repo root):
#   ./tooling/connect/build-swift-xcframework.sh
#
# Outputs (committed paths):
#   crates/spoke-connect/bindings/swift/generated/
#   crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework
#
# Coverage: one staticlib per Apple target triple, combined into an
# xcframework with three LibraryIdentifiers:
#   macos-arm64                 aarch64-apple-darwin    (host)
#   ios-arm64                   aarch64-apple-ios       (device)
#   ios-arm64_x86_64-simulator  aarch64-apple-ios-sim + x86_64-apple-ios
#                               (lipo'd: Apple Silicon + Intel simulators)
#
# xcodebuild -create-xcframework rejects two discrete -library entries for
# the same platform, so the two simulator staticlibs are lipo'd into one
# multi-arch slice before assembly.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "${REPO_ROOT}"

SWIFT_BINDINGS="${REPO_ROOT}/crates/spoke-connect/bindings/swift"
GENERATED="${SWIFT_BINDINGS}/generated"
XCFRAMEWORK="${SWIFT_BINDINGS}/xcframework/spoke_connectFFI.xcframework"

# CI affordance: when XCFRAMEWORK_OUTPUT_DIR is set, write the generated
# bindings and xcframework under it, leaving the committed trees untouched
# so the drift gate can compare committed vs built output.
if [[ -n "${XCFRAMEWORK_OUTPUT_DIR:-}" ]]; then
  GENERATED="${XCFRAMEWORK_OUTPUT_DIR}/generated"
  XCFRAMEWORK="${XCFRAMEWORK_OUTPUT_DIR}/spoke_connectFFI.xcframework"
fi

# CI affordance: --locked pins the committed Cargo.lock for reproducible CI
# builds; local nightly builds may run with a modified lockfile, so the flag
# stays off unless requested (XCFRAMEWORK_LOCKED=1).
LOCKED=()
if [[ "${XCFRAMEWORK_LOCKED:-0}" == "1" ]]; then
  LOCKED=(--locked)
fi

# Prefer nightly locally (AGENTS.md); fall back to default cargo (CI stable).
CARGO=(cargo)
RUSTUP_TOOLCHAIN=()
if command -v rustup >/dev/null 2>&1 && rustup toolchain list | grep -q '^nightly'; then
  CARGO=(cargo +nightly)
  RUSTUP_TOOLCHAIN=(--toolchain nightly)
fi

# Slice id -> Apple target triple. All four builds are explicit `--target`.
SLICES=(
  "macos-arm64|aarch64-apple-darwin"
  "ios-arm64|aarch64-apple-ios"
  "ios-arm64-simulator|aarch64-apple-ios-sim"
  "ios-x86_64-simulator|x86_64-apple-ios"
)

echo "==> assert Apple target triples installed"
if command -v rustup >/dev/null 2>&1; then
  # Assert against the same toolchain that builds (nightly locally, default in CI).
  # bash 3.2 (macOS system bash) treats an empty "${arr[@]}" under set -u as
  # an unbound variable; the + idiom expands to nothing when the array is
  # empty (the CI-stable path has no --toolchain flag).
  INSTALLED="$(rustup target list --installed ${RUSTUP_TOOLCHAIN[@]+"${RUSTUP_TOOLCHAIN[@]}"})"
  MISSING=()
  for entry in "${SLICES[@]}"; do
    triple="${entry#*|}"
    if ! grep -qx "${triple}" <<<"${INSTALLED}"; then
      MISSING+=("${triple}")
    fi
  done
  if [[ "${#MISSING[@]}" -gt 0 ]]; then
    echo "error: missing rustup targets for xcframework slices: ${MISSING[*]}" >&2
    if [[ "${#RUSTUP_TOOLCHAIN[@]}" -gt 0 ]]; then
      echo "install with: rustup target add --toolchain nightly ${MISSING[*]}" >&2
    else
      echo "install with: rustup target add ${MISSING[*]}" >&2
    fi
    exit 1
  fi
fi

TARGET_DIR="$("${CARGO[@]}" metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

FFI_FEATURES="ffi,remote-adapter"

echo "==> build ffi cdylib (bindgen metadata source; production surface — no ffi-smoke-host)"
"${CARGO[@]}" build ${LOCKED[@]+"${LOCKED[@]}"} -p spoke-connect --features "${FFI_FEATURES}" --release
CDYLIB="${TARGET_DIR}/release/libspoke_connect.dylib"
if [[ ! -f "${CDYLIB}" ]]; then
  echo "missing cdylib: ${CDYLIB}" >&2
  exit 1
fi

echo "==> generate Swift bindings"
mkdir -p "${GENERATED}"
"${CARGO[@]}" run ${LOCKED[@]+"${LOCKED[@]}"} -p spoke-connect --features bindgen-cli --bin uniffi-bindgen -- \
  generate --library "${CDYLIB}" \
  --language swift \
  --out-dir "${GENERATED}"

# Stage each staticlib under <tmp>/<slice-id>/; headers are shared across
# slices (xcodebuild copies them into each slice's Headers/).
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT
HDRS="${STAGE}/Headers"
mkdir -p "${HDRS}"
cp "${GENERATED}/spoke_connectFFI.h" "${HDRS}/"
cat > "${HDRS}/module.modulemap" <<'EOF'
module spoke_connectFFI {
    header "spoke_connectFFI.h"
    export *
}
EOF

echo "==> build staticlib slices"
for entry in "${SLICES[@]}"; do
  slice="${entry%%|*}"
  triple="${entry#*|}"
  echo "  -> ${slice} (${triple})"
  "${CARGO[@]}" rustc ${LOCKED[@]+"${LOCKED[@]}"} -p spoke-connect --features "${FFI_FEATURES}" --release --crate-type staticlib --target "${triple}"
  STATICLIB="${TARGET_DIR}/${triple}/release/libspoke_connect.a"
  if [[ ! -f "${STATICLIB}" ]]; then
    echo "missing staticlib: ${STATICLIB}" >&2
    exit 1
  fi
  mkdir -p "${STAGE}/${slice}"
  cp "${STATICLIB}" "${STAGE}/${slice}/"
done

echo "==> lipo simulator staticlibs into one multi-arch slice"
mkdir -p "${STAGE}/ios-simulator"
lipo -create \
  "${STAGE}/ios-arm64-simulator/libspoke_connect.a" \
  "${STAGE}/ios-x86_64-simulator/libspoke_connect.a" \
  -output "${STAGE}/ios-simulator/libspoke_connect.a"

echo "==> create xcframework (3 libraries, 4 target triples)"
rm -rf "${XCFRAMEWORK}"
mkdir -p "$(dirname "${XCFRAMEWORK}")"
xcodebuild -create-xcframework \
  -library "${STAGE}/macos-arm64/libspoke_connect.a" -headers "${HDRS}" \
  -library "${STAGE}/ios-arm64/libspoke_connect.a" -headers "${HDRS}" \
  -library "${STAGE}/ios-simulator/libspoke_connect.a" -headers "${HDRS}" \
  -output "${XCFRAMEWORK}"

# xcodebuild -create-xcframework emits AvailableLibraries in nondeterministic
# order (observed across identical invocations on the same machine/image),
# which would make the committed artifact drift from an otherwise identical
# CI rebuild. Normalize the ordering (sorted keys + LibraryIdentifier) so the
# build output is byte-deterministic; the drift gate compares real bytes.
echo "==> normalize Info.plist library ordering"
XCFRAMEWORK="${XCFRAMEWORK}" python3 - <<'PY'
import os
import plistlib

path = os.path.join(os.environ["XCFRAMEWORK"], "Info.plist")
with open(path, "rb") as f:
    plist = plistlib.load(f)
plist["AvailableLibraries"] = sorted(
    plist["AvailableLibraries"], key=lambda lib: lib["LibraryIdentifier"]
)
with open(path, "wb") as f:
    plistlib.dump(plist, f, sort_keys=True)
PY

echo "==> validate xcframework"
plutil -lint "${XCFRAMEWORK}/Info.plist"
for lib in "${XCFRAMEWORK}"/*/libspoke_connect.a; do
  echo "  -> $(lipo -info "${lib}")"
done

echo "==> done"
echo "  generated: ${GENERATED}"
echo "  xcframework: ${XCFRAMEWORK}"
echo "Validate: swift build (repo root), Smoke/README.md swiftc smoke, IosSmoke/README.md xcodebuild test"
