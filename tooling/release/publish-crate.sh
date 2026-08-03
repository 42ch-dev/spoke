#!/usr/bin/env bash
# Idempotent crates.io publish for one workspace package.
#
# - Skips when <crate>@<lockstep-version> is already on crates.io (safe re-run
#   after a partial publish-crates job).
# - Retries transient registry responses (502/503/504) with backoff.
# - Treats "already exists on crates.io" as success (publish race).
#
# Usage (repo root, CARGO_REGISTRY_TOKEN set):
#   ./tooling/release/publish-crate.sh spoke-schemas
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <package-name>" >&2
  exit 2
fi

PKG="$1"
MAX_ATTEMPTS="${CARGO_PUBLISH_MAX_ATTEMPTS:-5}"
SLEEP_SECS="${CARGO_PUBLISH_RETRY_SLEEP_SECS:-30}"
USER_AGENT="${CARGO_PUBLISH_USER_AGENT:-spoke-release/0.1 (github.com/42ch-dev/spoke)}"

VERSION="$(
  cargo metadata --no-deps --format-version 1 | python3 -c "
import json, sys
meta = json.load(sys.stdin)
name = sys.argv[1]
for p in meta['packages']:
    if p['name'] == name and p.get('source') is None:
        print(p['version'])
        break
else:
    raise SystemExit(f'workspace package not found: {name}')
" "${PKG}"
)"
if [[ -z "${VERSION}" ]]; then
  echo "failed to read workspace version for ${PKG}" >&2
  exit 1
fi

crates_io_has_version() {
  local code
  code="$(
    curl -sS -o /dev/null -w '%{http_code}' -A "${USER_AGENT}" \
      "https://crates.io/api/v1/crates/${PKG}/${VERSION}" || true
  )"
  [[ "${code}" == "200" ]]
}

if crates_io_has_version; then
  echo "${PKG}@${VERSION} already on crates.io — skip"
  exit 0
fi

attempt=1
while true; do
  echo "cargo publish -p ${PKG} (attempt ${attempt}/${MAX_ATTEMPTS})"
  set +e
  out="$(cargo publish -p "${PKG}" 2>&1)"
  status=$?
  set -e
  printf '%s\n' "${out}"

  if [[ "${status}" -eq 0 ]]; then
    exit 0
  fi

  if printf '%s\n' "${out}" | grep -q 'already exists on crates.io'; then
    echo "${PKG}@${VERSION} already on crates.io (publish race) — ok"
    exit 0
  fi

  if printf '%s\n' "${out}" | grep -Eqi 'got 502|got 503|got 504|timed out|timeout|temporarily unavailable'; then
    if (( attempt >= MAX_ATTEMPTS )); then
      echo "exhausted ${MAX_ATTEMPTS} attempts after transient registry failure" >&2
      exit "${status}"
    fi
    echo "transient registry failure; sleeping ${SLEEP_SECS}s before retry"
    sleep "${SLEEP_SECS}"
    attempt=$((attempt + 1))
    continue
  fi

  exit "${status}"
done
