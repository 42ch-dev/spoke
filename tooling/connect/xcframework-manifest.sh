#!/usr/bin/env bash
# Emit the xcframework transfer manifest: one "<sha256>  <relative-path>" line
# per file, sorted by path.
#
# Usage (from repo root):
#   ./tooling/connect/xcframework-manifest.sh <dir>
#
# This is the single generator for the artifact manifest format: the
# xcframework workflow writes it beside the uploaded bundle and
# apply-xcframework-artifact.sh consumes it with `shasum -a 256 -c` before and
# after the transfer.
#
# `shasum -a 256` is the BSD/macOS standard and the macOS-runner-locked choice —
# GNU coreutils' sha256sum is not guaranteed on macOS images, so keep this
# script on macOS runners.

set -euo pipefail

if [[ $# -ne 1 || ! -d "$1" ]]; then
  echo "usage: $0 <dir>" >&2
  exit 2
fi

# simplify: newline-separated paths assume no filenames with newlines. The
# xcframework layout is fixed (Info.plist, <slice>/libspoke_connect.a,
# Headers/*), so a byte-exact per-file manifest over LF-split names suffices.
(cd "$1" && find . -type f | LC_ALL=C sort | xargs shasum -a 256)
