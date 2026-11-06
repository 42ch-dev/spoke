#!/usr/bin/env bash
# Verify that a CI-built xcframework matches the committed one, file by file.
#
# Usage (from repo root):
#   ./tooling/connect/verify-xcframework-drift.sh <committed-dir> <built-dir>
#   ./tooling/connect/verify-xcframework-drift.sh --manifest <dir>
#
# The committed xcframework IS the baseline. When a `provenance.json` sidecar
# sits beside the committed bundle, this script also checks that the committed
# producer manifest still describes the committed bytes and that the record
# agrees about the manifest digest and inventory; a tree without a sidecar
# claims no provenance, so plain temporary/older invocations stay valid. For
# each tree it emits one
#   "<sha256>  <relative-path>"
# line per file (SHA-256 via `shasum -a 256`, paths sorted with LC_ALL=C),
# diffs the two manifests, prints the unified diff, and exits non-zero on any
# mismatch. Run after an LFS-smudged checkout so <committed-dir> holds real
# bytes, not LFS pointers.
#
# `shasum -a 256` is the BSD/macOS standard and the macOS-runner-locked choice —
# GNU coreutils' sha256sum is not guaranteed on macOS images, so keep this
# script on macOS runners.
#
# --manifest <dir> emits the same sorted per-file manifest on stdout and is
# the single generator for the artifact manifest format (the xcframework
# workflow's "Write xcframework hash manifest" step calls this mode;
# apply-xcframework-artifact.sh consumes it via `shasum -a 256 -c`).

set -euo pipefail

# simplify: newline-separated paths assume no filenames with newlines. The
# xcframework layout is fixed (Info.plist, <slice>/libspoke_connect.a,
# Headers/*), so a byte-exact per-file manifest over LF-split names suffices.
manifest() {
  local dir="$1"
  (cd "${dir}" && find . -type f | LC_ALL=C sort | xargs shasum -a 256)
}

if [[ "${1:-}" == "--manifest" ]]; then
  if [[ $# -ne 2 || ! -d "$2" ]]; then
    echo "usage: $0 --manifest <dir>" >&2
    exit 2
  fi
  manifest "$2"
  exit 0
fi

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <committed-dir> <built-dir>" >&2
  exit 2
fi

COMMITTED_DIR="$1"
BUILT_DIR="$2"

for dir in "${COMMITTED_DIR}" "${BUILT_DIR}"; do
  if [[ ! -d "${dir}" ]]; then
    echo "error: not a directory: ${dir}" >&2
    exit 2
  fi
done

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

manifest "${COMMITTED_DIR}" > "${WORK}/committed.sha256"
manifest "${BUILT_DIR}" > "${WORK}/built.sha256"

rc=0
if diff -u "${WORK}/committed.sha256" "${WORK}/built.sha256" > "${WORK}/drift.diff"; then
  echo "no drift: ${COMMITTED_DIR} matches ${BUILT_DIR} ($(wc -l < "${WORK}/committed.sha256" | tr -d ' ') files)"
else
  echo "drift detected between committed and built xcframework:" >&2
  cat "${WORK}/drift.diff" >&2
  echo "commit count: committed $(wc -l < "${WORK}/committed.sha256" | tr -d ' ') files, built $(wc -l < "${WORK}/built.sha256" | tr -d ' ') files" >&2
  rc=1
fi

# The two-tree compare above proves "committed == rebuilt". It says nothing
# about the provenance sidecar committed beside the bundle, which is the only
# durable statement about where those bytes came from: a hand-edited bundle, a
# replaced LFS object or a mix of two apply runs would leave a confident-looking
# record describing different content. A tree with no sidecar claims no
# provenance, so plain temporary/older invocations remain valid.
BUNDLE_PARENT="$(cd "$(dirname "${COMMITTED_DIR}")" && pwd)"
BUNDLE_NAME="$(basename "${COMMITTED_DIR}")"
PROVENANCE="${BUNDLE_PARENT}/provenance.json"
NAMED_MANIFEST="${BUNDLE_PARENT}/${BUNDLE_NAME}.sha256"

if [[ -f "${PROVENANCE}" ]]; then
  if ! SIDECAR="$(
    python3 - "${PROVENANCE}" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        entry = json.load(handle)["xcframework"]
    manifest = entry["manifest"]
    digest = entry["manifestSha256"]
except Exception as error:  # malformed, wrong shape or missing fields
    print("error: unreadable provenance record: %s" % error, file=sys.stderr)
    raise SystemExit(1)
print(manifest)
print(digest)
PY
  )"; then
    echo "error: malformed provenance sidecar ${PROVENANCE}" >&2
    rc=1
  else
    MANIFEST_NAME="${SIDECAR%%$'\n'*}"
    MANIFEST_SHA="${SIDECAR#*$'\n'}"
    MANIFEST_SHA="${MANIFEST_SHA%$'\n'}"
    if [[ ! "${MANIFEST_NAME}" =~ ^[A-Za-z0-9._-]+$ ]]; then
      echo "error: provenance names an unusable manifest path '${MANIFEST_NAME}'" >&2
      rc=1
    else
      MANIFEST_PATH="${BUNDLE_PARENT}/${MANIFEST_NAME}"
      if [[ ! -f "${MANIFEST_PATH}" ]]; then
        echo "error: provenance records manifest '${MANIFEST_NAME}', which is not committed beside the bundle" >&2
        rc=1
      else
        ACTUAL_SHA="$(shasum -a 256 "${MANIFEST_PATH}" | cut -d' ' -f1)"
        if [[ "${ACTUAL_SHA}" != "${MANIFEST_SHA}" ]]; then
          echo "error: manifest digest drift: record ${MANIFEST_SHA}, committed ${ACTUAL_SHA}" >&2
          rc=1
        fi
        if ! ( cd "${COMMITTED_DIR}" && shasum -a 256 -c "${MANIFEST_PATH}" ); then
          echo "error: committed bundle disagrees with the recorded producer manifest" >&2
          rc=1
        fi
        ( cd "${COMMITTED_DIR}" && find . -type f | LC_ALL=C sort ) > "${WORK}/tree.files"
        awk '{ $1 = ""; sub(/^ +/, ""); print }' "${MANIFEST_PATH}" \
          | LC_ALL=C sort > "${WORK}/manifest.files"
        if ! diff -u "${WORK}/manifest.files" "${WORK}/tree.files" > "${WORK}/inventory.diff"; then
          echo "error: committed bundle inventory disagrees with the recorded manifest:" >&2
          cat "${WORK}/inventory.diff" >&2
          rc=1
        fi
      fi
    fi
  fi
elif [[ -f "${NAMED_MANIFEST}" ]]; then
  echo "error: ${NAMED_MANIFEST} is committed without ${PROVENANCE} — incomplete provenance sidecar" >&2
  rc=1
fi

exit "${rc}"
