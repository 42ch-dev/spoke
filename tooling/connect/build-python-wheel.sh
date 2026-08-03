#!/usr/bin/env bash
# Build one platform wheel for the spoke-connect Python binding (wheel-only; no sdist).
#
# Usage (from repo root):
#   ./tooling/connect/build-python-wheel.sh
#   ./tooling/connect/build-python-wheel.sh --artifact-root <dir> --rid linux-x64
#
# <artifact-root> layout (same as build-connect-ffi / C# assemble):
#   linux-x64/libspoke_connect.so
#   win-x64/spoke_connect.dll
#   osx-arm64/libspoke_connect.dylib
#
# Without --artifact-root, uses the native already beside spoke_connect/ (host smoke layout).
# Writes wheels to crates/spoke-connect/bindings/python/dist/

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PY_ROOT="${REPO_ROOT}/crates/spoke-connect/bindings/python"
PKG="${PY_ROOT}/spoke_connect"
DIST="${PY_ROOT}/dist"

ARTIFACT_ROOT=""
RID=""
APPEND=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact-root)
      ARTIFACT_ROOT="$2"
      shift 2
      ;;
    --rid)
      RID="$2"
      shift 2
      ;;
    --append)
      APPEND=true
      shift
      ;;
    -h|--help)
      sed -n '2,15p' "$0"
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -n "${ARTIFACT_ROOT}" && -z "${RID}" ]]; then
  echo "--rid is required when --artifact-root is set" >&2
  exit 2
fi
if [[ -z "${ARTIFACT_ROOT}" && -n "${RID}" ]]; then
  echo "--artifact-root is required when --rid is set" >&2
  exit 2
fi

rid_to_native() {
  case "$1" in
    linux-x64) echo "libspoke_connect.so" ;;
    win-x64) echo "spoke_connect.dll" ;;
    osx-arm64) echo "libspoke_connect.dylib" ;;
    *)
      echo "unsupported rid: $1 (expected linux-x64, win-x64, osx-arm64)" >&2
      exit 1
      ;;
  esac
}

rid_to_plat_name() {
  case "$1" in
    linux-x64) echo "manylinux_2_35_x86_64" ;;
    win-x64) echo "win_amd64" ;;
    osx-arm64) echo "macosx_11_0_arm64" ;;
    *)
      echo "unsupported rid: $1 (expected linux-x64, win-x64, osx-arm64)" >&2
      exit 1
      ;;
  esac
}

detect_host_rid() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "${os}" in
    Darwin)
      if [[ "${arch}" == "arm64" ]]; then
        echo "osx-arm64"
      else
        echo "unsupported host: ${os}/${arch} (Python wheel recipe targets osx-arm64 on Apple Silicon)" >&2
        exit 1
      fi
      ;;
    Linux)
      if [[ "${arch}" == "x86_64" ]]; then
        echo "linux-x64"
      else
        echo "unsupported host: ${os}/${arch}" >&2
        exit 1
      fi
      ;;
    MINGW*|MSYS*|CYGWIN*)
      echo "win-x64"
      ;;
    *)
      echo "unsupported host OS: ${os}" >&2
      exit 1
      ;;
  esac
}

if [[ -z "${RID}" ]]; then
  RID="$(detect_host_rid)"
fi

if [[ -n "${ARTIFACT_ROOT}" ]]; then
  ARTIFACT_ROOT="$(cd "${ARTIFACT_ROOT}" && pwd)"
  native_name="$(rid_to_native "${RID}")"
  src="${ARTIFACT_ROOT}/${RID}/${native_name}"
  if [[ ! -f "${src}" ]]; then
    echo "missing native for ${RID}: ${src}" >&2
    exit 1
  fi
  # Remove other platform natives so the wheel carries exactly one RID library.
  rm -f "${PKG}/libspoke_connect.dylib" "${PKG}/libspoke_connect.so" "${PKG}/spoke_connect.dll"
  cp "${src}" "${PKG}/${native_name}"
  if [[ "${native_name}" == "libspoke_connect.dylib" ]] && command -v install_name_tool >/dev/null; then
    install_name_tool -id @rpath/libspoke_connect.dylib "${PKG}/${native_name}"
  fi
  echo "staged ${RID}/${native_name} beside spoke_connect module"
fi

native_count=0
for f in "${PKG}/libspoke_connect.dylib" "${PKG}/libspoke_connect.so" "${PKG}/spoke_connect.dll"; do
  if [[ -f "${f}" ]]; then
    native_count=$((native_count + 1))
  fi
done
if [[ "${native_count}" -ne 1 ]]; then
  echo "expected exactly one native library beside ${PKG}/__init__.py (found ${native_count})" >&2
  echo "stage via --artifact-root + --rid or copy host native before building" >&2
  exit 1
fi

if ! python3 -c "import setuptools" 2>/dev/null; then
  echo "setuptools is required (pip install setuptools)" >&2
  exit 1
fi

mkdir -p "${DIST}"
if [[ "${APPEND}" != true ]]; then
  rm -f "${DIST}"/*.whl
fi

plat_name="$(rid_to_plat_name "${RID}")"
echo "building wheel for rid=${RID} plat-name=${plat_name}"

export SPOKE_PYTHON_WHEEL_RID="${RID}"
python3 -m pip wheel "${PY_ROOT}" \
  --no-deps \
  --wheel-dir "${DIST}"

echo "wheels under ${DIST}:"
ls -la "${DIST}"/*.whl
