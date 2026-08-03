#!/usr/bin/env bash
# Fail closed if release wheel inventory is incomplete or uses Warehouse-invalid tags.
#
# Usage (from repo root):
#   ./tooling/connect/verify-python-wheels.sh [dist-dir]
#
# Expects exactly three wheels for spoke-connect lockstep version with PEP 425 tags:
#   py3-none-manylinux_2_35_x86_64
#   py3-none-macosx_11_0_arm64
#   py3-none-win_amd64

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIST="${1:-${REPO_ROOT}/crates/spoke-connect/bindings/python/dist}"
VERSION="$(node -p "require('${REPO_ROOT}/package.json').version")"

LINUX_TAG="manylinux_2_35_x86_64"
MAC_TAG="macosx_11_0_arm64"
WIN_TAG="win_amd64"

if [[ ! -d "${DIST}" ]]; then
  echo "missing dist directory: ${DIST}" >&2
  exit 1
fi

wheels=()
while IFS= read -r line; do
  wheels+=("${line}")
done < <(find "${DIST}" -maxdepth 1 -name '*.whl' -print | sort)
if [[ "${#wheels[@]}" -ne 3 ]]; then
  echo "expected 3 wheels in ${DIST}, found ${#wheels[@]}" >&2
  ls -la "${DIST}" || true
  exit 1
fi

check_wheel() {
  local plat_tag="$1"
  local native_file="$2"
  local match=""
  for w in "${wheels[@]}"; do
    local base
    base="$(basename "${w}")"
    if [[ "${base}" == *"-py3-none-${plat_tag}.whl" ]]; then
      match="${w}"
      break
    fi
  done
  if [[ -z "${match}" ]]; then
    echo "missing wheel with platform tag py3-none-${plat_tag}" >&2
    exit 1
  fi
  if [[ "$(basename "${match}")" != spoke_connect-"${VERSION}"-* ]]; then
    echo "wheel version mismatch (expected ${VERSION}): ${match}" >&2
    exit 1
  fi
  if ! unzip -l "${match}" | grep -q "spoke_connect/__init__.py"; then
    echo "wheel missing spoke_connect/__init__.py: ${match}" >&2
    exit 1
  fi
  if ! unzip -l "${match}" | grep -q "spoke_connect/${native_file}"; then
    echo "wheel missing spoke_connect/${native_file}: ${match}" >&2
    exit 1
  fi
  echo "OK ${match}"
}

for w in "${wheels[@]}"; do
  if [[ "$(basename "${w}")" == *"linux_x86_64"* ]]; then
    echo "warehouse-invalid platform tag linux_x86_64 in ${w}" >&2
    exit 1
  fi
done

check_wheel "${LINUX_TAG}" "libspoke_connect.so"
check_wheel "${MAC_TAG}" "libspoke_connect.dylib"
check_wheel "${WIN_TAG}" "spoke_connect.dll"

echo "wheel inventory OK for spoke-connect ${VERSION}"
