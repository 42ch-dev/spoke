#!/usr/bin/env bash
# Assemble spoke-connect FFI natives into Kotlin JNA resource layout.
#
# Maps release.yml build-connect-ffi RIDs to JNA classpath prefixes (contract
# connect-binding-channels.md §3.5):
#   linux-x64  -> linux-x86-64/libspoke_connect.so
#   win-x64    -> win32-x86-64/spoke_connect.dll
#   osx-arm64   -> darwin-aarch64/libspoke_connect.dylib
#
# Writes into:
#   crates/spoke-connect/bindings/kotlin/src/main/resources/<jna-rid>/
#
# Usage (from repo root):
#   ./tooling/connect/assemble-kotlin-natives.sh <artifact-root>
#
# <artifact-root> contains one directory per ffi-matrix RID (linux-x64, win-x64,
# osx-arm64), each holding the native lib basename from release.yml.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KOTLIN_ROOT="${REPO_ROOT}/crates/spoke-connect/bindings/kotlin"
RESOURCES_ROOT="${KOTLIN_ROOT}/src/main/resources"

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <artifact-root>" >&2
  exit 2
fi

ARTIFACT_ROOT="$(cd "$1" && pwd)"

mkdir -p "${RESOURCES_ROOT}"

copy_one() {
  local ffi_rid="$1"
  local jna_rid="$2"
  local name="$3"
  local src="${ARTIFACT_ROOT}/${ffi_rid}/${name}"
  if [[ ! -f "${src}" ]]; then
    echo "missing native for ${ffi_rid}: ${src}" >&2
    exit 1
  fi
  local dest_dir="${RESOURCES_ROOT}/${jna_rid}"
  mkdir -p "${dest_dir}"
  cp "${src}" "${dest_dir}/${name}"
  echo "assembled ${jna_rid}/${name} (from ffi ${ffi_rid})"
}

if [[ -d "${ARTIFACT_ROOT}/linux-x64" ]]; then
  copy_one linux-x64 linux-x86-64 libspoke_connect.so
fi
if [[ -d "${ARTIFACT_ROOT}/win-x64" ]]; then
  copy_one win-x64 win32-x86-64 spoke_connect.dll
fi
if [[ -d "${ARTIFACT_ROOT}/osx-arm64" ]]; then
  copy_one osx-arm64 darwin-aarch64 libspoke_connect.dylib
fi

if [[ -z "$(find "${RESOURCES_ROOT}" -type f 2>/dev/null | head -1)" ]]; then
  echo "no JNA natives assembled under ${ARTIFACT_ROOT}" >&2
  exit 1
fi

echo "JNA resources ready under ${RESOURCES_ROOT}"
