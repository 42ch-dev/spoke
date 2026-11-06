#!/usr/bin/env bash
# Apply a CI-built xcframework artifact to the committed tree in one command.
#
# Usage (from repo root, on a feature branch with a clean worktree):
#   ./tooling/connect/apply-xcframework-artifact.sh <run-id> [--allow-sha <sha>]
#
# `<run-id>` is the numeric Actions run id from the run URL — a bare URL is
# rejected before any network call, because only the number can be recorded in
# the provenance sidecar.
#
# Downloads the xcframework artifact from <run-id>, verifies the run's
# provenance (workflow, conclusion, headSha), checksum-verifies the artifact
# against its mandatory hash manifest, validates the producer record the CI
# now ships inside the artifact (its sourceHead must equal the run head and
# its embedded manifest must equal the artifact manifest byte-for-byte, which
# also yields the observed compiler version), rsyncs the artifact over the
# committed tree (LFS pointers staged via .gitattributes clean filter), writes
# the sibling `provenance.json` + manifest copy from the run's own facts plus
# that observed compiler, and prints the suggested commit line. Requires `gh`,
# `rsync` and `python3` (the record is emitted with `json.dump` and re-parsed
# before staging). Run as a maintainer with normal credentials — never
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

# `gh` also accepts a run URL, but only the numeric id can be embedded in the
# provenance record. Reject every other form here — before the first network
# call — so a malformed value can never reach the record writer.
if [[ ! "${RUN_ID}" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: run id must be the numeric Actions run id, got '${RUN_ID}'" >&2
  echo "       usage: $0 <run-id> [--allow-sha <sha>]" >&2
  exit 2
fi
if [[ -n "${ALLOW_SHA}" && ! "${ALLOW_SHA}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "error: --allow-sha expects a full 40-character commit sha, got '${ALLOW_SHA}'" >&2
  exit 2
fi
ALLOW_SHA_USED=""

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
if ! command -v python3 >/dev/null 2>&1; then
  echo "error: python3 required (provenance record emission and validation)" >&2
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
    ALLOW_SHA_USED="${ALLOW_SHA}"
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

# The producer now ships its own record inside the artifact (the workflow's
# "Record xcframework build provenance" step). It must agree with the run and
# with the manifest bytes before the apply proceeds; the observed compiler it
# carries is real producer evidence, kept separate from the declared recipe.
PRODUCER_RECORD="${ARTIFACT_ROOT}/provenance.json"
if [[ ! -f "${PRODUCER_RECORD}" ]]; then
  echo "error: artifact has no provenance.json from the producer" >&2
  exit 1
fi
if ! OBSERVED_RUSTC="$(
  PRODUCER_RECORD="${PRODUCER_RECORD}" RUN_HEAD_SHA="${RUN_HEAD_SHA}" MANIFEST="${MANIFEST}" python3 - <<'PY'
import json
import os
import sys

try:
    with open(os.environ["PRODUCER_RECORD"], encoding="utf-8") as handle:
        record = json.load(handle)
except Exception as error:  # missing, malformed or wrong shape
    print("error: unreadable artifact provenance: %s" % error, file=sys.stderr)
    raise SystemExit(1)
if record.get("sourceHead") != os.environ["RUN_HEAD_SHA"]:
    print(
        "error: artifact provenance sourceHead %s != run head %s"
        % (record.get("sourceHead"), os.environ["RUN_HEAD_SHA"]),
        file=sys.stderr,
    )
    raise SystemExit(1)
with open(os.environ["MANIFEST"], encoding="utf-8") as handle:
    if record.get("manifest") != handle.read():
        print("error: artifact provenance manifest does not match the artifact manifest", file=sys.stderr)
        raise SystemExit(1)
rustc = record.get("rustcVersion")
if not isinstance(rustc, str) or not rustc:
    print("error: artifact provenance carries no observed rustcVersion", file=sys.stderr)
    raise SystemExit(1)
print(rustc)
PY
)"; then
  echo "error: artifact provenance does not match the run and manifest" >&2
  exit 1
fi

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

# Persist the producer's manifest and the run that produced these bytes beside
# the bundle — never inside it, so the drift comparison's file inventory stays
# exactly the ten built files. The Actions artifact expires after five days;
# this record is the durable trace of what the committed bytes came from.
#
# The declared recipe (toolchain pin, Apple targets, --locked) is read from the
# workflow file at the RUN's own head, so it states what the producer declared
# rather than what this checkout happens to say now. The actual compiler/Xcode
# banner is deliberately not recorded: the run log does not print it, and
# writing a version here would be invention.
SIDECAR_DIR="$(dirname "${XCFRAMEWORK_DIR}")"
MANIFEST_NAME="$(basename "${MANIFEST}")"
MANIFEST_SHA="$(shasum -a 256 "${MANIFEST}" | cut -d' ' -f1)"
cp "${MANIFEST}" "${SIDECAR_DIR}/${MANIFEST_NAME}"

RECIPE_SOURCE=".github/workflows/xcframework.yml"
if ! RECIPE_YAML="$(git show "${RUN_HEAD_SHA}:${RECIPE_SOURCE}" 2>/dev/null)"; then
  echo "error: cannot read ${RECIPE_SOURCE} at the run head ${RUN_HEAD_SHA}" >&2
  exit 1
fi
DECLARED_TOOLCHAIN=""
DECLARED_TARGETS=""
DECLARED_LOCKED=""
if ! RECIPE="$(
  RECIPE_YAML="${RECIPE_YAML}" python3 - <<'PY'
import os
import re

text = os.environ["RECIPE_YAML"]
toolchain = re.search(r"^\s*toolchain:\s*(\S+)\s*$", text, re.M)
targets = re.search(r"^\s*targets:\s*(.+?)\s*$", text, re.M)
locked = re.search(r'^\s*XCFRAMEWORK_LOCKED:\s*"?([01])"?\s*$', text, re.M)
if not (toolchain and targets and locked):
    raise SystemExit(1)
print(toolchain.group(1))
print(targets.group(1))
print(locked.group(1))
PY
)"; then
  echo "error: ${RECIPE_SOURCE}@${RUN_HEAD_SHA} does not declare toolchain/targets/locked inputs" >&2
  exit 1
fi
{ IFS= read -r DECLARED_TOOLCHAIN; IFS= read -r DECLARED_TARGETS; IFS= read -r DECLARED_LOCKED; } <<< "${RECIPE}"
if [[ -z "${DECLARED_TOOLCHAIN}" || -z "${DECLARED_TARGETS}" || -z "${DECLARED_LOCKED}" ]]; then
  echo "error: ${RECIPE_SOURCE}@${RUN_HEAD_SHA} does not declare toolchain/targets/locked inputs" >&2
  exit 1
fi

RECORD="${SIDECAR_DIR}/provenance.json"
RUN_ID="${RUN_ID}" RUN_HEAD_SHA="${RUN_HEAD_SHA}" RUN_CONCLUSION="${RUN_CONCLUSION}" \
APPLIED_AT_HEAD="${EXPECT_SHA}" ALLOW_SHA_USED="${ALLOW_SHA_USED}" \
ARTIFACT_NAME="${ARTIFACT_NAME}" WF_NAME="${WF_NAME}" RECIPE_SOURCE="${RECIPE_SOURCE}" \
MANIFEST_NAME="${MANIFEST_NAME}" MANIFEST_SHA="${MANIFEST_SHA}" \
DECLARED_TOOLCHAIN="${DECLARED_TOOLCHAIN}" DECLARED_TARGETS="${DECLARED_TARGETS}" \
DECLARED_LOCKED="${DECLARED_LOCKED}" OBSERVED_RUSTC="${OBSERVED_RUSTC}" python3 - "${RECORD}" <<'PY'
import json
import os
import sys

env = os.environ
record = {
    "schemaVersion": 1,
    "xcframework": {
        "artifact": env["ARTIFACT_NAME"],
        "workflow": env["WF_NAME"],
        "runId": int(env["RUN_ID"]),
        "runConclusion": env["RUN_CONCLUSION"],
        "headSha": env["RUN_HEAD_SHA"],
        "appliedAtHead": env["APPLIED_AT_HEAD"],
        "allowSha": env["ALLOW_SHA_USED"] or None,
        "declaredRecipe": {
            "source": "%s@%s" % (env["RECIPE_SOURCE"], env["RUN_HEAD_SHA"]),
            "toolchain": env["DECLARED_TOOLCHAIN"],
            "targets": [item.strip() for item in env["DECLARED_TARGETS"].split(",")],
            "locked": env["DECLARED_LOCKED"] == "1",
        },
        "observedRustc": env["OBSERVED_RUSTC"],
        "manifest": env["MANIFEST_NAME"],
        "manifestSha256": env["MANIFEST_SHA"],
    },
}
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump(record, handle, indent=2)
    handle.write("\n")
PY

# Re-parse the written file: a record that cannot be read back must never be
# staged as the durable trace.
python3 -m json.tool "${RECORD}" >/dev/null

git add "${SIDECAR_DIR}/${MANIFEST_NAME}" "${RECORD}"

echo "==> staged changes:"
git status --short -- "${XCFRAMEWORK_DIR}" "${SIDECAR_DIR}/${MANIFEST_NAME}" "${SIDECAR_DIR}/provenance.json"
echo
echo "Suggested commit line:"
echo "  build(connect): refresh xcframework from CI artifact ${RUN_ID}"
