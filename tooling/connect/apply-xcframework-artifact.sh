#!/usr/bin/env bash
# Apply a CI-built xcframework artifact to the committed tree in one command.
#
# Usage (from repo root, on a feature branch with a clean worktree):
#   ./tooling/connect/apply-xcframework-artifact.sh <run-id> [--allow-sha <sha>]
#
# Downloads the xcframework artifact from <run-id>, verifies the run's
# provenance (workflow, conclusion, headSha), checksum-verifies the artifact
# against its mandatory hash manifest, rsyncs it over the committed tree
# (LFS pointers staged via .gitattributes clean filter), and prints the
# suggested commit line. Run as a maintainer with normal credentials — never
# GITHUB_TOKEN (token pushes don't re-trigger workflows and can't carry LFS
# objects).
#
# Provenance gate (fail-closed, runs before the download): the run must be
# from the `Xcframework` workflow, must have concluded `success` — or
# `failure` whose only failed step is the drift gate, which still ships a
# complete, manifest-bearing artifact (that is the drift-gate flow's
# contract: a drift-red run's build + manifest steps ran green) — and its
# headSha must match the current checkout HEAD. Any other conclusion
# (cancelled, timed out, build/upload failure, ...) is rejected: the artifact
# may be missing or incomplete. `--allow-sha <sha>` overrides the HEAD match
# for deliberately pinning an older build (loud warning).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "${REPO_ROOT}"

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <run-id> [--allow-sha <sha>]" >&2
  exit 2
fi
RUN_ID="$1"
shift
ALLOW_SHA=""
if [[ $# -eq 2 && "${1:-}" == "--allow-sha" ]]; then
  ALLOW_SHA="$2"
elif [[ $# -ne 0 ]]; then
  echo "usage: $0 <run-id> [--allow-sha <sha>]" >&2
  exit 2
fi

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

# Keep in sync with the workflow name and drift step name in
# .github/workflows/xcframework.yml.
WF_NAME="Xcframework"
DRIFT_STEP_NAME="Verify drift vs committed xcframework"

RUN_META="$(gh run view "${RUN_ID}" --json workflowName,headSha,status,conclusion --template '{{.workflowName}}|{{.headSha}}|{{.status}}|{{.conclusion}}')"
IFS='|' read -r RUN_WORKFLOW RUN_HEAD_SHA RUN_STATUS RUN_CONCLUSION <<< "${RUN_META}"

if [[ "${RUN_STATUS}" != "completed" ]]; then
  echo "error: run ${RUN_ID} has status '${RUN_STATUS}' (not completed) — nothing to apply yet" >&2
  exit 1
fi
if [[ "${RUN_WORKFLOW}" != "${WF_NAME}" ]]; then
  echo "error: run ${RUN_ID} is from workflow '${RUN_WORKFLOW}', expected '${WF_NAME}' — refusing to apply" >&2
  exit 1
fi

case "${RUN_CONCLUSION}" in
  success)
    ;;
  failure)
    # A failure caused only by the drift gate still ships the full artifact:
    # build + manifest steps ran green before the gate, and the upload step
    # is unconditional. Any other failed step (build, manifest, upload, ...)
    # means the artifact may be missing or incomplete — reject.
    FAILED_STEPS="$(gh run view "${RUN_ID}" --json jobs --template '{{range .jobs}}{{range .steps}}{{if eq .conclusion "failure"}}{{.name}}{{println}}{{end}}{{end}}{{end}}')"
    DRIFT_ONLY=1
    while IFS= read -r step; do
      if [[ -n "${step}" && "${step}" != "${DRIFT_STEP_NAME}" ]]; then
        DRIFT_ONLY=0
        break
      fi
    done <<< "${FAILED_STEPS}"
    if [[ "${DRIFT_ONLY}" -eq 1 && -n "${FAILED_STEPS}" ]]; then
      echo "==> run ${RUN_ID} failed only at the drift gate — artifact is still a valid apply input"
    else
      echo "error: run ${RUN_ID} failed with steps other than the drift gate:" >&2
      printf '%s\n' "${FAILED_STEPS}" >&2
      exit 1
    fi
    ;;
  *)
    echo "error: run ${RUN_ID} conclusion '${RUN_CONCLUSION}' is not acceptable" >&2
    echo "       (expected 'success', or 'failure' at the drift gate only)" >&2
    exit 1
    ;;
esac

EXPECT_SHA="$(git rev-parse HEAD)"
if [[ "${RUN_HEAD_SHA}" != "${EXPECT_SHA}" ]]; then
  if [[ -n "${ALLOW_SHA}" && "${RUN_HEAD_SHA}" == "${ALLOW_SHA}" ]]; then
    echo "WARNING: applying run ${RUN_ID} built at ${RUN_HEAD_SHA} to a checkout at ${EXPECT_SHA}" >&2
    echo "WARNING: the committed artifact will NOT match the sources at this HEAD —" >&2
    echo "WARNING: only use --allow-sha to deliberately pin an older build" >&2
  else
    echo "error: run ${RUN_ID} was built at ${RUN_HEAD_SHA}, checkout HEAD is ${EXPECT_SHA}" >&2
    echo "       apply from a checkout at the run's SHA, or pass --allow-sha ${RUN_HEAD_SHA} to override" >&2
    exit 1
  fi
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

# Mandatory checksum verification (fail-closed): no manifest, no apply.
MANIFEST="${ARTIFACT_ROOT}/spoke_connectFFI.xcframework.sha256"
if [[ ! -f "${MANIFEST}" ]]; then
  echo "error: artifact ${ARTIFACT_NAME} has no hash manifest (spoke_connectFFI.xcframework.sha256)" >&2
  echo "       refusing to apply an unverifiable artifact — use a run whose manifest step ran" >&2
  ls -la "${ARTIFACT_ROOT}" >&2
  exit 1
fi
echo "==> verify artifact against hash manifest"
(cd "${BUILT}" && shasum -a 256 -c "${MANIFEST}")

echo "==> rsync artifact over committed tree"
rsync -a --delete "${BUILT}/" "${XCFRAMEWORK_DIR}/"

# LFS guard: without git-lfs the .gitattributes clean filter is missing and
# `git add` would stage raw ~120 MB binaries instead of pointers.
echo "==> stage LFS pointers"
if ! git lfs version >/dev/null 2>&1; then
  echo "error: git-lfs required so .a files stage as LFS pointers, not raw binaries" >&2
  echo "       (brew install git-lfs && git lfs install)" >&2
  exit 1
fi
LFS_TARGET="$(find "${XCFRAMEWORK_DIR}" -name '*.a' -print -quit)"
if [[ -z "${LFS_TARGET}" ]]; then
  echo "error: no .a file under ${XCFRAMEWORK_DIR} — nothing for git-lfs to track" >&2
  exit 1
fi
if [[ "$(git check-attr filter -- "${LFS_TARGET}")" != *"filter: lfs"* ]]; then
  echo "error: .gitattributes does not map ${LFS_TARGET} to filter=lfs — refusing to stage raw binaries" >&2
  exit 1
fi

git add "${XCFRAMEWORK_DIR}"

echo "==> staged changes:"
git status --short -- "${XCFRAMEWORK_DIR}"
echo
echo "Suggested commit line:"
echo "  build(connect): refresh xcframework from CI artifact ${RUN_ID}"
