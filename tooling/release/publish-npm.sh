#!/usr/bin/env bash
# Idempotent npm publish for one workspace package (OIDC Trusted Publisher).
set -euo pipefail
if [[ $# -ne 2 ]]; then
  echo "usage: $0 <package-directory> <npm-name>" >&2
  exit 2
fi
PKG_DIR="$1"
NPM_NAME="$2"
VERSION="$(node -p "require('./${PKG_DIR}/package.json').version")"
if npm view "${NPM_NAME}@${VERSION}" version >/dev/null 2>&1; then
  echo "${NPM_NAME}@${VERSION} already on npm — skip"
  exit 0
fi
(
  cd "${PKG_DIR}"
  pnpm pack
  set +e
  out="$(npm publish ./*.tgz --access public 2>&1)"
  status=$?
  set -e
  printf '%s\n' "${out}"
  if [[ "${status}" -eq 0 ]]; then
    exit 0
  fi
  if printf '%s\n' "${out}" | grep -q 'cannot publish over the previously published versions'; then
    echo "${NPM_NAME}@${VERSION} already on npm (publish race) — ok"
    exit 0
  fi
  exit "${status}"
)
