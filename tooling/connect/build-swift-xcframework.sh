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

# Prefer nightly locally (AGENTS.md); the locked/CI path builds with the
# workflow-pinned default toolchain exactly (XCFRAMEWORK_LOCKED=1 opts into
# --locked builds and the pinned toolchain — never +nightly, so a runner
# image that ships nightly cannot bypass the pin).
CARGO=(cargo)
RUSTUP_TOOLCHAIN=()
if [[ "${XCFRAMEWORK_LOCKED:-0}" != "1" ]] \
  && command -v rustup >/dev/null 2>&1 \
  && rustup toolchain list | grep -q '^nightly'; then
  CARGO=(cargo +nightly)
  RUSTUP_TOOLCHAIN=(--toolchain nightly)
fi

# Cargo folds the package version into crate metadata and therefore into every
# mangled symbol. Keep the native artifact independent of the lockstep version
# for the duration of the build, then restore every edited manifest/lockfile.
CARGO_VERSION_SENTINEL="${XCFRAMEWORK_VERSION_SENTINEL:-0.0.0}"
CARGO_VERSION_BACKUP=""
XCFRAMEWORK_VERSION_LOCK="${REPO_ROOT}/.xcframework-version.lock"
XCFRAMEWORK_VERSION_LOCK_HELD=0
acquire_xcframework_version_lock() {
  local holder_pid stale_lock
  while ! mkdir "${XCFRAMEWORK_VERSION_LOCK}" 2>/dev/null; do
    holder_pid=""
    if [[ -f "${XCFRAMEWORK_VERSION_LOCK}/pid" ]]; then
      IFS= read -r holder_pid < "${XCFRAMEWORK_VERSION_LOCK}/pid" || true
    fi
    if [[ "${holder_pid}" =~ ^[1-9][0-9]*$ ]] \
      && kill -0 "${holder_pid}" 2>/dev/null; then
      echo "error: another xcframework build (PID ${holder_pid}) is already normalising Cargo versions in this checkout; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to queue" >&2
      return 1
    fi
    stale_lock="${XCFRAMEWORK_VERSION_LOCK}.stale.$$"
    if ! mv "${XCFRAMEWORK_VERSION_LOCK}" "${stale_lock}" 2>/dev/null; then
      continue
    fi
    if [[ -n "${holder_pid}" ]]; then
      echo "warning: removing stale xcframework version lock (dead PID ${holder_pid}): ${XCFRAMEWORK_VERSION_LOCK}" >&2
    else
      echo "warning: removing stale xcframework version lock (missing holder PID): ${XCFRAMEWORK_VERSION_LOCK}" >&2
    fi
    rm -rf "${stale_lock}"
  done
  XCFRAMEWORK_VERSION_LOCK_HELD=1
  printf '%s\n' "$$" > "${XCFRAMEWORK_VERSION_LOCK}/pid"
}
release_xcframework_version_lock() {
  if [[ "${XCFRAMEWORK_VERSION_LOCK_HELD}" -eq 1 ]]; then
    rm -rf "${XCFRAMEWORK_VERSION_LOCK}"
    XCFRAMEWORK_VERSION_LOCK_HELD=0
  fi
}
restore_cargo_versions() {
  local status=0
  local previous_int previous_term backup_path
  previous_int="$(trap -p INT || true)"
  previous_term="$(trap -p TERM || true)"
  trap '' INT TERM
  if [[ -n "${CARGO_VERSION_BACKUP}" && -f "${CARGO_VERSION_BACKUP}/files.json" ]]; then
    backup_path="$(cd "${CARGO_VERSION_BACKUP}" && pwd -P)"
    if ! node "${REPO_ROOT}/tooling/connect/normalize-cargo-version.mjs" restore \
      "${REPO_ROOT}" "${CARGO_VERSION_BACKUP}"; then
      echo "error: failed to restore Cargo version surfaces; backup retained at ${backup_path}" >&2
      status=1
    else
      rm -rf "${CARGO_VERSION_BACKUP}"
    fi
  elif [[ -n "${CARGO_VERSION_BACKUP}" ]]; then
    rm -rf "${CARGO_VERSION_BACKUP}"
  fi
  rm -rf "${STAGE:-}"
  if [[ -n "${previous_int}" ]]; then
    eval "${previous_int}"
  else
    trap - INT
  fi
  if [[ -n "${previous_term}" ]]; then
    eval "${previous_term}"
  else
    trap - TERM
  fi
  return "${status}"
}
cleanup() {
  local status=$?
  trap - EXIT
  trap '' INT TERM
  if ! restore_cargo_versions; then
    status=1
  fi
  release_xcframework_version_lock
  exit "${status}"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
acquire_xcframework_version_lock
CARGO_VERSION_BACKUP="$(mktemp -d)"
node "${REPO_ROOT}/tooling/connect/normalize-cargo-version.mjs" apply \
  "${REPO_ROOT}" "${CARGO_VERSION_BACKUP}" "${CARGO_VERSION_SENTINEL}"

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
