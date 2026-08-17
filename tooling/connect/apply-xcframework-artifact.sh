#!/usr/bin/env bash
# Apply a CI-built xcframework artifact to the committed tree in one command.
#
# Usage (from repo root, on a feature branch with a clean worktree):
#   ./tooling/connect/apply-xcframework-artifact.sh <run-id>
#
# Downloads the xcframework artifact from <run-id>, rsyncs it over the
# committed tree (LFS pointers staged via .gitattributes clean filter), and
# prints the suggested commit line. Run as a maintainer with normal
# credentials — never GITHUB_TOKEN (token pushes don't re-trigger workflows
# and can't carry LFS objects).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "${REPO_ROOT}"

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <run-id>" >&2
  exit 2
fi
RUN_ID="$1"

# Keep in sync with the artifact name in .github/workflows/xcframework.yml.
ARTIFACT_NAME="spoke-connect-xcframework"
XCFRAMEWORK_DIR="crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework"

if ! command -v gh >/dev/null 2>&1; then
  echo "error: gh CLI required (brew install gh)" >&2
  exit 1
fi
if ! command -v rsync >/dev/null 2>&1; then
  echo "error: rsync required" >&2
  exit 1
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT

echo "==> download artifact ${ARTIFACT_NAME} from run ${RUN_ID}"
gh run download "${RUN_ID}" --name "${ARTIFACT_NAME}" --dir "${STAGE}"

# gh extracts a single named artifact under <dir>/<name>/; accept the layout
# where it lands directly in <dir>.
ARTIFACT_ROOT="${STAGE}"
if [[ -d "${STAGE}/${ARTIFACT_NAME}" ]]; then
  ARTIFACT_ROOT="${STAGE}/${ARTIFACT_NAME}"
fi

BUILT="${ARTIFACT_ROOT}/spoke_connectFFI.xcframework"
if [[ ! -f "${BUILT}/Info.plist" ]]; then
  echo "error: artifact ${ARTIFACT_NAME} has no spoke_connectFFI.xcframework/Info.plist" >&2
  ls -la "${ARTIFACT_ROOT}" >&2
  exit 1
fi

MANIFEST="${ARTIFACT_ROOT}/spoke_connectFFI.xcframework.sha256"
if [[ -f "${MANIFEST}" ]]; then
  echo "==> verify artifact against hash manifest"
  (cd "${BUILT}" && shasum -a 256 -c "${MANIFEST}")
else
  echo "warning: no hash manifest in artifact — skipping checksum verification" >&2
fi

echo "==> rsync artifact over committed tree"
rsync -a --delete "${BUILT}/" "${XCFRAMEWORK_DIR}/"

echo "==> stage LFS pointers"
git add "${XCFRAMEWORK_DIR}"

echo "==> staged changes:"
git status --short -- "${XCFRAMEWORK_DIR}"
echo
echo "Suggested commit line:"
echo "  build(connect): refresh xcframework from CI artifact ${RUN_ID}"
