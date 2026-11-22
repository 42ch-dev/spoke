#!/usr/bin/env bash
# Regenerate committed Swift bindings + multi-slice xcframework for SPM.
#
# Usage (from repo root):
#   ./tooling/connect/build-swift-xcframework.sh [--phase <host|slice|assemble|all>]
#                                                [--slice-id <id>] [--target <triple>]
#
# Outputs (committed paths):
#   crates/spoke-connect/bindings/swift/generated/
#   crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework
#
# Coverage: one staticlib per Apple target triple, combined into an
# xcframework with three LibraryIdentifiers:
#   macos-arm64                 aarch64-apple-darwin    (= host triple on Apple Silicon)
#   ios-arm64                   aarch64-apple-ios       (device)
#   ios-arm64_x86_64-simulator  aarch64-apple-ios-sim + x86_64-apple-ios
#                               (lipo'd: Apple Silicon + Intel simulators)
#
# xcodebuild -create-xcframework rejects two discrete -library entries for
# the same platform, so the two simulator staticlibs are lipo'd into one
# multi-arch slice before assembly.
#
# The host cdylib + uniffi-bindgen build runs for the machine's host triple
# (derived from `rustc -vV`) in the shared target dir. The host build and the
# macOS slice share dependency artifacts when the compiler's host triple equals
# the slice triple (aarch64-apple-darwin).
# All four staticlib slices then build sequentially in that same target dir.
#
# Phases (CI splits the pipeline into one job per phase; default `all`
# preserves the single-invocation behaviour for local runs):
#   host      normalize Cargo versions, build the host cdylib +
#             uniffi-bindgen, generate Swift bindings + headers/modulemap into
#             $XCFRAMEWORK_OUTPUT_DIR/{generated,Headers}. No slices, no
#             xcodebuild.
#   slice     normalize Cargo versions, build one staticlib for
#             --slice-id/--target, copy it to
#             $XCFRAMEWORK_OUTPUT_DIR/slices/<slice-id>/libspoke_connect.a.
#             Requires --slice-id and --target; no bindgen, no xcodebuild.
#   assemble  lipo the simulator slices in $XCFRAMEWORK_OUTPUT_DIR/slices/,
#             run xcodebuild -create-xcframework with
#             $XCFRAMEWORK_OUTPUT_DIR/Headers, normalize Info.plist and
#             validate. No cargo.
#   all       host + every slice + assemble (the historical local path).

set -euo pipefail

usage() {
  sed -n '2,/^set -euo pipefail$/p' "$0" | sed '$d'
}

PHASE="all"
SLICE_ID=""
SLICE_TARGET=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --phase)
      [[ $# -ge 2 ]] || { echo "error: --phase requires a value" >&2; exit 2; }
      PHASE="$2"; shift 2 ;;
    --slice-id)
      [[ $# -ge 2 ]] || { echo "error: --slice-id requires a value" >&2; exit 2; }
      SLICE_ID="$2"; shift 2 ;;
    --target)
      [[ $# -ge 2 ]] || { echo "error: --target requires a value" >&2; exit 2; }
      SLICE_TARGET="$2"; shift 2 ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "error: unknown argument '$1'" >&2; usage >&2; exit 2 ;;
  esac
done
case "${PHASE}" in
  host|slice|assemble|all) ;;
  *)
    echo "error: --phase must be one of host|slice|assemble|all, got '${PHASE}'" >&2
    exit 2 ;;
esac
if [[ -n "${SLICE_ID}${SLICE_TARGET}" && "${PHASE}" != "slice" ]]; then
  echo "error: --slice-id/--target only apply to --phase slice" >&2
  exit 2
fi
if [[ "${PHASE}" == "slice" && ( -z "${SLICE_ID}" || -z "${SLICE_TARGET}" ) ]]; then
  echo "error: --phase slice requires --slice-id <id> and --target <triple>" >&2
  exit 2
fi

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "${REPO_ROOT}"

SWIFT_BINDINGS="${REPO_ROOT}/crates/spoke-connect/bindings/swift"
GENERATED="${SWIFT_BINDINGS}/generated"
XCFRAMEWORK="${SWIFT_BINDINGS}/xcframework/spoke_connectFFI.xcframework"

# CI affordance: stage output without modifying the committed consumer package.
if [[ -n "${XCFRAMEWORK_OUTPUT_DIR:-}" ]]; then
  GENERATED="${XCFRAMEWORK_OUTPUT_DIR}/generated"
  XCFRAMEWORK="${XCFRAMEWORK_OUTPUT_DIR}/spoke_connectFFI.xcframework"
fi

# Slice id -> Apple target triple. All four builds are explicit `--target`.
SLICES=(
  "macos-arm64|aarch64-apple-darwin"
  "ios-arm64|aarch64-apple-ios"
  "ios-arm64-simulator|aarch64-apple-ios-sim"
  "ios-x86_64-simulator|x86_64-apple-ios"
)

# Directory holding <slice-id>/libspoke_connect.a inputs. The `all` phase stages
# into mktemp as before; `slice` writes and `assemble` reads under
# $XCFRAMEWORK_OUTPUT_DIR/slices/.
SLICES_DIR=""
if [[ -n "${XCFRAMEWORK_OUTPUT_DIR:-}" ]]; then
  SLICES_DIR="${XCFRAMEWORK_OUTPUT_DIR}/slices"
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
RUSTC_TOOLCHAIN=()
if [[ "${XCFRAMEWORK_LOCKED:-0}" != "1" ]] \
  && command -v rustup >/dev/null 2>&1 \
  && rustup toolchain list | grep -q '^nightly'; then
  CARGO=(cargo +nightly)
  RUSTUP_TOOLCHAIN=(--toolchain nightly)
  RUSTC_TOOLCHAIN=(+nightly)
fi

# Cargo folds the package version into crate metadata and therefore into every
# mangled symbol. Keep the native artifact independent of the lockstep version
# for the duration of the build, then restore every edited manifest/lockfile.
CARGO_VERSION_SENTINEL="${XCFRAMEWORK_VERSION_SENTINEL:-0.0.0}"
CARGO_VERSION_BACKUP=""
# target/ is already gitignored, so the lock does not dirty the checkout.
XCFRAMEWORK_VERSION_LOCK="${REPO_ROOT}/target/.xcframework-version.lock"
XCFRAMEWORK_VERSION_LOCK_HELD=0
create_xcframework_version_lock() {
  (set -C; printf '%s\n' "$$" > "${XCFRAMEWORK_VERSION_LOCK}") 2>/dev/null
}
acquire_xcframework_version_lock() {
  local holder_pid holder_process ps_status stale_lock moved_holder
  if ! mkdir -p "$(dirname "${XCFRAMEWORK_VERSION_LOCK}")" 2>/dev/null; then
    echo "error: cannot create xcframework version lock directory; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
    return 1
  fi
  if create_xcframework_version_lock; then
    XCFRAMEWORK_VERSION_LOCK_HELD=1
    return 0
  fi

  if [[ ! -e "${XCFRAMEWORK_VERSION_LOCK}" ]]; then
    echo "error: cannot create xcframework version lock; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
    return 1
  fi
  if [[ ! -f "${XCFRAMEWORK_VERSION_LOCK}" || -L "${XCFRAMEWORK_VERSION_LOCK}" ]]; then
    echo "error: cannot safely inspect xcframework version lock; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to clear it" >&2
    return 1
  fi
  holder_pid=""
  if ! IFS= read -r holder_pid < "${XCFRAMEWORK_VERSION_LOCK}" && [[ -z "${holder_pid}" ]]; then
    echo "error: cannot read xcframework version lock holder identity; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to clear it" >&2
    return 1
  fi
  if [[ ! "${holder_pid}" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: cannot determine xcframework version lock holder; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to clear it" >&2
    return 1
  fi
  if kill -0 "${holder_pid}" 2>/dev/null; then
    echo "error: another xcframework build (PID ${holder_pid}) is already normalising Cargo versions in this checkout; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to queue" >&2
    return 1
  fi
  holder_process=""
  ps_status=0
  holder_process="$(ps -p "${holder_pid}" -o pid= 2>/dev/null)" || ps_status=$?
  if [[ "${holder_process}" =~ [0-9] ]]; then
    echo "error: another xcframework build (PID ${holder_pid}) is already normalising Cargo versions in this checkout; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to queue" >&2
    return 1
  fi
  if [[ "${ps_status}" -ne 1 ]]; then
    echo "error: cannot confirm xcframework version lock holder PID ${holder_pid} is dead; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to clear it" >&2
    return 1
  fi

  stale_lock="${XCFRAMEWORK_VERSION_LOCK}.stale.$$"
  if [[ -e "${stale_lock}" ]]; then
    echo "error: cannot remove confirmed-dead xcframework version lock safely; stale path exists: ${stale_lock}" >&2
    return 1
  fi
  if ! mv "${XCFRAMEWORK_VERSION_LOCK}" "${stale_lock}" 2>/dev/null; then
    echo "error: cannot remove confirmed-dead xcframework version lock; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
    return 1
  fi
  moved_holder=""
  if ! IFS= read -r moved_holder < "${stale_lock}" && [[ -z "${moved_holder}" ]]; then
    echo "error: xcframework version lock changed while clearing it; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to remove the replacement" >&2
    if [[ ! -e "${XCFRAMEWORK_VERSION_LOCK}" ]]; then
      mv "${stale_lock}" "${XCFRAMEWORK_VERSION_LOCK}" 2>/dev/null || true
    fi
    return 1
  fi
  if [[ "${moved_holder}" != "${holder_pid}" ]]; then
    echo "error: xcframework version lock changed while clearing it; lock: ${XCFRAMEWORK_VERSION_LOCK}; refusing to remove the replacement" >&2
    if [[ ! -e "${XCFRAMEWORK_VERSION_LOCK}" ]]; then
      mv "${stale_lock}" "${XCFRAMEWORK_VERSION_LOCK}" 2>/dev/null || true
    fi
    return 1
  fi
  if ! rm -f "${stale_lock}"; then
    echo "error: cannot remove confirmed-dead xcframework version lock; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
    return 1
  fi
  echo "warning: removing stale xcframework version lock (dead PID ${holder_pid}): ${XCFRAMEWORK_VERSION_LOCK}" >&2
  if ! create_xcframework_version_lock; then
    echo "error: cannot acquire xcframework version lock after clearing confirmed-dead holder; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
    return 1
  fi
  XCFRAMEWORK_VERSION_LOCK_HELD=1
}
release_xcframework_version_lock() {
  local holder_pid release_lock moved_holder
  if [[ "${XCFRAMEWORK_VERSION_LOCK_HELD}" -eq 1 ]]; then
    holder_pid=""
    if [[ ! -f "${XCFRAMEWORK_VERSION_LOCK}" ]] \
      || ! IFS= read -r holder_pid < "${XCFRAMEWORK_VERSION_LOCK}" \
      && [[ -z "${holder_pid}" ]]; then
      echo "warning: leaving xcframework version lock in place because its holder identity cannot be read; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
      XCFRAMEWORK_VERSION_LOCK_HELD=0
      return
    fi
    if [[ "${holder_pid}" != "$$" ]]; then
      echo "warning: leaving xcframework version lock in place because it is held by PID ${holder_pid}; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
      XCFRAMEWORK_VERSION_LOCK_HELD=0
      return
    fi
    release_lock="${XCFRAMEWORK_VERSION_LOCK}.release.$$"
    if [[ -e "${release_lock}" ]] \
      || ! mv "${XCFRAMEWORK_VERSION_LOCK}" "${release_lock}" 2>/dev/null; then
      echo "warning: could not remove owned xcframework version lock; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
      XCFRAMEWORK_VERSION_LOCK_HELD=0
      return
    fi
    moved_holder=""
    if ! IFS= read -r moved_holder < "${release_lock}" && [[ -z "${moved_holder}" ]]; then
      echo "warning: leaving replacement xcframework version lock in place; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
      if [[ ! -e "${XCFRAMEWORK_VERSION_LOCK}" ]]; then
        mv "${release_lock}" "${XCFRAMEWORK_VERSION_LOCK}" 2>/dev/null || true
      fi
      XCFRAMEWORK_VERSION_LOCK_HELD=0
      return
    fi
    if [[ "${moved_holder}" != "$$" ]]; then
      echo "warning: leaving replacement xcframework version lock in place; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
      if [[ ! -e "${XCFRAMEWORK_VERSION_LOCK}" ]]; then
        mv "${release_lock}" "${XCFRAMEWORK_VERSION_LOCK}" 2>/dev/null || true
      fi
      XCFRAMEWORK_VERSION_LOCK_HELD=0
      return
    fi
    if ! rm -f "${release_lock}"; then
      echo "warning: could not remove owned xcframework version lock; lock: ${XCFRAMEWORK_VERSION_LOCK}" >&2
    fi
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

# Every cargo phase (host, slice, all) normalizes the lockstep version before
# compiling and restores it via the EXIT trap. The assemble phase never
# compiles, so it skips the lock and the sentinel entirely.
normalize_cargo_versions() {
  acquire_xcframework_version_lock
  CARGO_VERSION_BACKUP="$(mktemp -d)"
  node "${REPO_ROOT}/tooling/connect/normalize-cargo-version.mjs" apply \
    "${REPO_ROOT}" "${CARGO_VERSION_BACKUP}" "${CARGO_VERSION_SENTINEL}"
}

# Assert the given triples are installed on the toolchain that will build
# (nightly locally, default in CI). bash 3.2 (macOS system bash) treats an
# empty "${arr[@]}" under set -u as an unbound variable; the + idiom expands
# to nothing when the array is empty (the CI-stable path has no --toolchain
# flag).
assert_rustup_targets() {
  local entry triple
  command -v rustup >/dev/null 2>&1 || return 0
  local INSTALLED
  INSTALLED="$(rustup target list --installed ${RUSTUP_TOOLCHAIN[@]+"${RUSTUP_TOOLCHAIN[@]}"})"
  local MISSING=()
  for triple in "$@"; do
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
}

cargo_target_dir() {
  "${CARGO[@]}" metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])'
}

FFI_FEATURES="ffi,remote-adapter"

# An explicit RUSTC is probed as-is (no toolchain flag); only the rustup proxy
# may take +nightly.
host_triple() {
  local probe=("${RUSTC:-rustc}") triple
  if [[ -z "${RUSTC:-}" ]] && [[ "${#RUSTC_TOOLCHAIN[@]}" -gt 0 ]]; then
    probe+=("${RUSTC_TOOLCHAIN[@]}")
  fi
  triple="$("${probe[@]}" -vV | sed -n 's/^host: //p')"
  if [[ -z "${triple}" ]]; then
    echo "error: could not determine the toolchain host triple from rustc -vV" >&2
    exit 1
  fi
  printf '%s' "${triple}"
}

# Phase `host` entry point; the build itself is shared with `all`.
phase_host() {
  normalize_cargo_versions
  phase_host_body
}

# Host cdylib + uniffi-bindgen, Swift bindings into $GENERATED, and the shared
# headers/modulemap — into $XCFRAMEWORK_OUTPUT_DIR/Headers for the standalone
# phase, or the staging dir's Headers/ under `all`. One body so the two paths
# cannot drift apart.
phase_host_body() {
  local target_dir host host_build_dir
  target_dir="$(cargo_target_dir)"
  host="$(host_triple)"
  host_build_dir="${target_dir}/${host}/release"

  echo "==> build ffi cdylib + uniffi-bindgen (bindgen metadata source; production surface — no ffi-smoke-host)"
  "${CARGO[@]}" build ${LOCKED[@]+"${LOCKED[@]}"} -p spoke-connect --features "${FFI_FEATURES},bindgen-cli" --release --target "${host}"
  local cdylib="${host_build_dir}/libspoke_connect.dylib"
  if [[ ! -f "${cdylib}" ]]; then
    echo "missing cdylib: ${cdylib}" >&2
    exit 1
  fi

  echo "==> generate Swift bindings"
  mkdir -p "${GENERATED}"
  "${host_build_dir}/uniffi-bindgen" generate --library "${cdylib}" \
    --language swift \
    --out-dir "${GENERATED}"

  stage_headers "$(headers_dir)"
}

stage_headers() {
  local hdrs="$1"
  mkdir -p "${hdrs}"
  cp "${GENERATED}/spoke_connectFFI.h" "${hdrs}/"
  cat > "${hdrs}/module.modulemap" <<'EOF'
module spoke_connectFFI {
    header "spoke_connectFFI.h"
    export *
}
EOF
}

# One header tree for every phase: the staged output dir when set, otherwise
# the invocation's own staging dir (phase `all`). Producer and consumer phases
# resolve through here so the two paths cannot drift.
headers_dir() {
  if [[ -n "${XCFRAMEWORK_OUTPUT_DIR:-}" ]]; then
    printf '%s\n' "${XCFRAMEWORK_OUTPUT_DIR}/Headers"
  else
    printf '%s\n' "${STAGE}/Headers"
  fi
}

# Phase `slice`: one staticlib for --slice-id/--target into
# $SLICES_DIR/<slice-id>/libspoke_connect.a.
phase_slice() {
  local expected=""
  local entry
  for entry in "${SLICES[@]}"; do
    if [[ "${entry%%|*}" == "${SLICE_ID}" ]]; then
      expected="${entry#*|}"
      break
    fi
  done
  if [[ -z "${expected}" ]]; then
    echo "error: unknown slice id '${SLICE_ID}' (known: $(printf '%s ' "${SLICES[@]%%|*}"))" >&2
    exit 2
  fi
  if [[ "${SLICE_TARGET}" != "${expected}" ]]; then
    echo "error: slice '${SLICE_ID}' builds for '${expected}', not '${SLICE_TARGET}'" >&2
    exit 2
  fi
  if [[ -z "${SLICES_DIR}" ]]; then
    echo "error: --phase slice requires XCFRAMEWORK_OUTPUT_DIR" >&2
    exit 2
  fi

  normalize_cargo_versions
  assert_rustup_targets "${SLICE_TARGET}"
  local target_dir
  target_dir="$(cargo_target_dir)"
  build_slice_staticlib "${SLICE_ID}" "${SLICE_TARGET}" "${target_dir}" "${SLICES_DIR}/${SLICE_ID}"
}

build_slice_staticlib() {
  local slice="$1" triple="$2" target_dir="$3" dest="$4"
  echo "  -> ${slice} (${triple})"
  "${CARGO[@]}" rustc ${LOCKED[@]+"${LOCKED[@]}"} -p spoke-connect --features "${FFI_FEATURES}" --release --crate-type staticlib --target "${triple}"
  local staticlib="${target_dir}/${triple}/release/libspoke_connect.a"
  if [[ ! -f "${staticlib}" ]]; then
    echo "missing staticlib: ${staticlib}" >&2
    exit 1
  fi
  mkdir -p "${dest}"
  cp "${staticlib}" "${dest}/"
}

# Phase `assemble`: lipo the two simulator staticlibs, create the xcframework,
# normalize Info.plist and validate. Reads $SLICES_DIR + the Headers tree the
# host phase produced; never touches cargo.
phase_assemble() {
  if [[ -z "${SLICES_DIR}" || -z "${XCFRAMEWORK_OUTPUT_DIR:-}" ]]; then
    echo "error: --phase assemble requires XCFRAMEWORK_OUTPUT_DIR" >&2
    exit 2
  fi
  local hdrs="$(headers_dir)"
  if [[ ! -f "${hdrs}/spoke_connectFFI.h" ]]; then
    echo "error: missing host headers: ${hdrs}/spoke_connectFFI.h" >&2
    exit 1
  fi
  STAGE="${STAGE:-$(mktemp -d)}"
  assemble_xcframework "${SLICES_DIR}" "${hdrs}"
}

assemble_xcframework() {
  local slices_dir="$1" hdrs="$2"

  echo "==> lipo simulator staticlibs into one multi-arch slice"
  mkdir -p "${STAGE}/ios-simulator"
  lipo -create \
    "${slices_dir}/ios-arm64-simulator/libspoke_connect.a" \
    "${slices_dir}/ios-x86_64-simulator/libspoke_connect.a" \
    -output "${STAGE}/ios-simulator/libspoke_connect.a"

  echo "==> create xcframework (3 libraries, 4 target triples)"
  rm -rf "${XCFRAMEWORK}"
  mkdir -p "$(dirname "${XCFRAMEWORK}")"
  xcodebuild -create-xcframework \
    -library "${slices_dir}/macos-arm64/libspoke_connect.a" -headers "${hdrs}" \
    -library "${slices_dir}/ios-arm64/libspoke_connect.a" -headers "${hdrs}" \
    -library "${STAGE}/ios-simulator/libspoke_connect.a" -headers "${hdrs}" \
    -output "${XCFRAMEWORK}"

  # Normalize xcodebuild's nondeterministic AvailableLibraries ordering.
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
  validate_xcframework_slices "${XCFRAMEWORK}"
}

# Check the exact consumer slice set and architectures, including missing slices.
EXPECTED_SLICES=(ios-arm64 ios-arm64_x86_64-simulator macos-arm64)

slice_architectures() {
  local slice="$1" lib="$2" expected actual
  case "${slice}" in
    macos-arm64 | ios-arm64) expected="arm64" ;;
    ios-arm64_x86_64-simulator) expected="arm64 x86_64" ;;
    *)
      echo "error: unexpected xcframework slice '${slice}'" >&2
      return 1
      ;;
  esac
  if [[ ! -f "${lib}" ]]; then
    echo "error: missing slice ${slice} (no ${slice}/libspoke_connect.a)" >&2
    return 1
  fi
  actual="$(lipo -archs "${lib}" | tr ' ' '\n' | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//')"
  if [[ "${actual}" != "${expected}" ]]; then
    echo "error: ${slice} contains [${actual}], expected [${expected}]" >&2
    return 1
  fi
  echo "  -> ${slice}: ${actual}"
}

validate_xcframework_slices() {
  local bundle="$1" slice entry
  for slice in "${EXPECTED_SLICES[@]}"; do
    slice_architectures "${slice}" "${bundle}/${slice}/libspoke_connect.a" || return 1
  done
  for entry in "${bundle}"/*/; do
    slice="$(basename "${entry}")"
    case " ${EXPECTED_SLICES[*]} " in
      *" ${slice} "*) ;;
      *)
        echo "error: unexpected xcframework slice '${slice}'" >&2
        return 1
        ;;
    esac
  done
}

# Phase `all`: the historical single-invocation path — host work, every slice
# in sequence into a staging dir, then assembly. Equivalent to running the
# three phases locally but keeps the mktemp staging layout.
phase_all() {
  normalize_cargo_versions

  echo "==> assert Apple target triples installed"
  local all_triples=()
  local entry
  for entry in "${SLICES[@]}"; do
    all_triples+=("${entry#*|}")
  done
  assert_rustup_targets "${all_triples[@]}"

  STAGE="$(mktemp -d)"
  phase_host_body
  local target_dir
  target_dir="$(cargo_target_dir)"
  echo "==> build staticlib slices (shared target dir)"
  for entry in "${SLICES[@]}"; do
    build_slice_staticlib "${entry%%|*}" "${entry#*|}" "${target_dir}" "${STAGE}/${entry%%|*}"
  done

  assemble_xcframework "${STAGE}" "$(headers_dir)"
}


# Dispatch. `phase_host` wraps the shared body for the single-phase CI job.
case "${PHASE}" in
  host)     phase_host ;;
  slice)    phase_slice ;;
  assemble) phase_assemble ;;
  all)      phase_all ;;
esac

echo "==> done"
echo "  generated: ${GENERATED}"
echo "  xcframework: ${XCFRAMEWORK}"
echo "Validate: swift build (repo root), Smoke/README.md swiftc smoke, IosSmoke/README.md xcodebuild test"
