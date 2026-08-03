#!/usr/bin/env bash
# Build all ffi-matrix platform wheels for spoke-connect (release / CI).
#
# Usage (from repo root):
#   ./tooling/connect/build-python-wheels-from-ffi.sh <artifact-root>
#
# <artifact-root> matches build-connect-ffi / NuGet assemble layout:
#   linux-x64/libspoke_connect.so
#   win-x64/spoke_connect.dll
#   osx-arm64/libspoke_connect.dylib
#
# Writes three wheels to crates/spoke-connect/bindings/python/dist/

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <artifact-root>" >&2
  exit 2
fi

ARTIFACT_ROOT="$(cd "$1" && pwd)"
BUILD_ONE="${SCRIPT_DIR}/build-python-wheel.sh"

for rid in linux-x64 win-x64 osx-arm64; do
  args=(--artifact-root "${ARTIFACT_ROOT}" --rid "${rid}")
  if [[ "${rid}" != "linux-x64" ]]; then
    args+=(--append)
  fi
  "${BUILD_ONE}" "${args[@]}"
done

echo "release wheels:"
ls -la "${SCRIPT_DIR}/../../crates/spoke-connect/bindings/python/dist/"*.whl
