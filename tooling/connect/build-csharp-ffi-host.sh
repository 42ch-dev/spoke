#!/usr/bin/env bash
# Build spoke-connect --features ffi,remote-adapter --release for the host RID and stage
# under a directory suitable for assemble-csharp-runtimes.sh.
#
# Usage (from repo root):
#   ./tooling/connect/build-csharp-ffi-host.sh [staging-dir]
#
# Default staging-dir: /tmp/spoke-connect-ffi-artifacts

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STAGING="${1:-/tmp/spoke-connect-ffi-artifacts}"

cd "${REPO_ROOT}"

# Prefer nightly locally (AGENTS.md); fall back to default cargo (CI stable).
CARGO=(cargo)
if command -v rustup >/dev/null 2>&1 && rustup toolchain list | grep -q '^nightly'; then
  CARGO=(cargo +nightly)
fi

echo "building spoke-connect ffi release with: ${CARGO[*]}"
"${CARGO[@]}" build -p spoke-connect --features ffi,remote-adapter --release

uname_s="$(uname -s)"
uname_m="$(uname -m)"
case "${uname_s}" in
  Linux)
    rid=linux-x64
    libname=libspoke_connect.so
    ;;
  Darwin)
    if [[ "${uname_m}" == "arm64" ]]; then
      rid=osx-arm64
    else
      rid=osx-x64
    fi
    libname=libspoke_connect.dylib
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    rid=win-x64
    libname=spoke_connect.dll
    ;;
  *)
    echo "unsupported host: ${uname_s}/${uname_m}" >&2
    exit 1
    ;;
esac

# CARGO_TARGET_DIR may relocate artifacts (direnv / CI).
TARGET_DIR="$("${CARGO[@]}" metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
SRC="${TARGET_DIR}/release/${libname}"
if [[ ! -f "${SRC}" ]]; then
  echo "missing release native: ${SRC}" >&2
  exit 1
fi

dest_dir="${STAGING}/${rid}"
mkdir -p "${dest_dir}"
cp "${SRC}" "${dest_dir}/${libname}"
echo "staged ${rid}/${libname} -> ${dest_dir}/${libname}"
echo "${STAGING}"
