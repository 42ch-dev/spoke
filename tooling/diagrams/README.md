# Diagrams

Source specifications for the SVG diagrams used by `README.md` and the docs site.

| Spec | Output |
|------|--------|
| `spoke-overview.architecture.json` | `docs/public/diagrams/spoke-overview.{light,dark}.svg` |
| `protocol-layers.architecture.json` | `protocol-layers.{light,dark}.svg` |
| `adapter-promote.sequence.json` | `adapter-promote.{light,dark}.svg` |
| `connect-session.sequence.json` | `connect-session.{light,dark}.svg` |
| `knowledge-entry-status.lifecycle.json` | `knowledge-entry-status.{light,dark}.svg` |
| `protocol-pipeline.dataflow.json` | `protocol-pipeline.{light,dark}.svg` |

## Regenerate

The diagrams are authored and validated with the [Archify](https://github.com/tt-a1i/archify) skill. Keep the skill outside the repository.

```bash
# 1. Validate the specification
node "$ARCHIFY_SKILL/bin/archify.mjs" validate architecture tooling/diagrams/specs/spoke-overview.architecture.json --quality showcase --json

# 2. Deliver the interactive HTML into a scratch directory
node "$ARCHIFY_SKILL/bin/archify.mjs" deliver architecture tooling/diagrams/specs/spoke-overview.architecture.json /tmp/spoke-overview.html --quality showcase --json

# 3. Export static light/dark SVGs for README and VitePress
ARCHIFY_SKILL="$ARCHIFY_SKILL" ARCHIFY_CHROME="/path/to/chrome" \
  node tooling/diagrams/export-svg.mjs /tmp/spoke-overview.html docs/public/diagrams
```

Use `validate --quality showcase` for every type. The export helper requires Chrome or Chromium; set `ARCHIFY_CHROME` to its executable. It inlines computed presentation styles, resolves the theme background, and writes two standalone SVGs. The interactive HTML is not committed; only the SVGs are.
