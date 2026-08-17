#!/usr/bin/env bash
# Verify that a CI-built xcframework matches the committed one, file by file.
#
# Usage (from repo root):
#   ./tooling/connect/verify-xcframework-drift.sh <committed-dir> <built-dir>
#   ./tooling/connect/verify-xcframework-drift.sh --manifest <dir>
#
# The committed xcframework IS the baseline (no separate manifest file is
# committed). For each tree this script emits one
#   "<sha256>  <relative-path>"
# line per file (SHA-256 via `shasum -a 256`, paths sorted with LC_ALL=C),
# diffs the two manifests, prints the unified diff, and exits non-zero on any
# mismatch. Run after an LFS-smudged checkout so <committed-dir> holds real
# bytes, not LFS pointers.
#
# `shasum -a 256` is the BSD/macOS standard and the macos-14-locked choice —
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

if diff -u "${WORK}/committed.sha256" "${WORK}/built.sha256" > "${WORK}/drift.diff"; then
  echo "no drift: ${COMMITTED_DIR} matches ${BUILT_DIR} ($(wc -l < "${WORK}/committed.sha256" | tr -d ' ') files)"
  exit 0
fi

echo "drift detected between committed and built xcframework:" >&2
cat "${WORK}/drift.diff" >&2
echo "commit count: committed $(wc -l < "${WORK}/committed.sha256" | tr -d ' ') files, built $(wc -l < "${WORK}/built.sha256" | tr -d ' ') files" >&2
exit 1
