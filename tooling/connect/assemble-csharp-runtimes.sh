# Assemble spoke-connect FFI natives into the C# NuGet RID layout.

# Usage (from repo root):
#   ./tooling/connect/assemble-csharp-runtimes.sh <artifact-root>
#
# <artifact-root> contains one directory per RID, each holding the native lib:
#   linux-x64/libspoke_connect.so
#   win-x64/spoke_connect.dll
#   osx-arm64/libspoke_connect.dylib
#
# Writes into:
#   crates/spoke-connect/bindings/csharp/runtimes/<rid>/native/

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CSHARP_ROOT="${REPO_ROOT}/crates/spoke-connect/bindings/csharp"
RUNTIMES_ROOT="${CSHARP_ROOT}/runtimes"

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <artifact-root>" >&2
  exit 2
fi

ARTIFACT_ROOT="$(cd "$1" && pwd)"

rm -rf "${RUNTIMES_ROOT}"
mkdir -p "${RUNTIMES_ROOT}"

copy_one() {
  local rid="$1"
  local name="$2"
  local src="${ARTIFACT_ROOT}/${rid}/${name}"
  if [[ ! -f "${src}" ]]; then
    echo "missing native for ${rid}: ${src}" >&2
    exit 1
  fi
  local dest_dir="${RUNTIMES_ROOT}/${rid}/native"
  mkdir -p "${dest_dir}"
  cp "${src}" "${dest_dir}/${name}"
  echo "assembled ${rid}/native/${name}"
}

# Release matrix RIDs (osx-x64 omitted — no macos-13 CI runner).
if [[ -d "${ARTIFACT_ROOT}/linux-x64" ]]; then
  copy_one linux-x64 libspoke_connect.so
fi
if [[ -d "${ARTIFACT_ROOT}/win-x64" ]]; then
  copy_one win-x64 spoke_connect.dll
fi
if [[ -d "${ARTIFACT_ROOT}/osx-arm64" ]]; then
  copy_one osx-arm64 libspoke_connect.dylib
fi

if [[ -z "$(find "${RUNTIMES_ROOT}" -type f 2>/dev/null | head -1)" ]]; then
  echo "no RID natives assembled under ${ARTIFACT_ROOT}" >&2
  exit 1
fi

echo "runtimes ready under ${RUNTIMES_ROOT}"
