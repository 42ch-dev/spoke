#!/usr/bin/env bash
# Post-generate patch for uniffi Kotlin:
# 1. CoreException / FfiException payload fields named `message` conflict with
#    kotlin.Throwable.message — rename to `detail` in Kotlin only.
# 2. LoopbackTransport / LoopbackSmokeHost / RemoteAdapterFfi export a domain
#    `close()` that collides with uniffi's Disposable AutoCloseable `close()`.
#
# Usage (from repo root):
#   ./crates/spoke-connect/bindings/kotlin/bindgen/patch-kotlin-core-error-fields.sh \
#     crates/spoke-connect/bindings/kotlin/generated/uniffi/spoke_connect/spoke_connect.kt

set -euo pipefail

TARGET="${1:-crates/spoke-connect/bindings/kotlin/generated/uniffi/spoke_connect/spoke_connect.kt}"

if [[ ! -f "${TARGET}" ]]; then
  echo "missing generated Kotlin: ${TARGET}" >&2
  exit 1
fi

perl -pi -e '
  s/val `message`: kotlin\.String/val `detail`: kotlin.String/g;
  s/get\(\) = "message=\$\{ `message` \}"/get() = "detail=\${ `detail` }"/g;
  s/get\(\) = "kind=\$\{ `kind` \}, message=\$\{ `message` \}"/get() = "kind=\${ `kind` }, detail=\${ `detail` }"/g;
  s/get\(\) = "code=\$\{ `code` \}, message=\$\{ `message` \}, kind=\$\{ `kind` \}, wireCode=\$\{ `wireCode` \}"/get() = "code=\${ `code` }, detail=\${ `detail` }, kind=\${ `kind` }, wireCode=\${ `wireCode` }"/g;
  s/value\.`message`/value.`detail`/g;
  s/fun `close`\(\)/fun close()/g;
' "${TARGET}"

python3 - "${TARGET}" <<'PY'
import re
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as f:
    content = f.read()

disposable_close = """
    @Synchronized
    override fun close() {
        this.destroy()
    }
"""

for class_name in ("LoopbackTransport", "LoopbackSmokeHost", "RemoteAdapterFfi"):
    marker = f"open class {class_name}:"
    start = content.find(marker)
    if start == -1:
        continue
    # Scope to this class body (next top-level `open class` / `public object` at column 0).
    rest = content[start:]
    end_match = re.search(r"\n(?=open class |public object |sealed class )", rest[len(marker) :])
    end = start + len(marker) + (end_match.start() if end_match else len(rest))
    block = content[start:end]
    if disposable_close not in block:
        continue
    block = block.replace(disposable_close, "\n", 1)
  # Domain close() should stay synchronized like other resource teardown.
    block = re.sub(
        r"(\n    )((?:@Throws\([^\n]+\)\s*)?override fun close\(\))",
        r"\1@Synchronized\n    \2",
        block,
        count=1,
    )
    content = content[:start] + block + content[end:]

with open(path, "w", encoding="utf-8") as f:
    f.write(content)
PY

echo "patched Kotlin bindgen output: ${TARGET}"
