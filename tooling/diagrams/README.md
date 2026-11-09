# Diagrams

Source specifications for the SVG diagrams used by `README.md` and the docs site. The SVGs are committed; the interactive HTML is a scratch artifact.

| Spec | Output |
|------|--------|
| `spoke-overview.architecture.json` | `docs/public/diagrams/spoke-overview.{light,dark}.svg` |
| `protocol-layers.architecture.json` | `protocol-layers.{light,dark}.svg` |
| `adapter-promote.sequence.json` | `adapter-promote.{light,dark}.svg` |
| `connect-session.sequence.json` | `connect-session.{light,dark}.svg` |
| `knowledge-entry-status.lifecycle.json` | `knowledge-entry-status.{light,dark}.svg` |
| `protocol-pipeline.dataflow.json` | `protocol-pipeline.{light,dark}.svg` |

`pnpm docs:diagrams` gates the committed set: every spec must have both SVGs, and each SVG must carry a `viewBox`, a `<title>`, and no `<script>`. `docs:build` runs it before VitePress.

## Regenerate

The diagrams are authored and validated with the [Archify](https://github.com/tt-a1i/archify) skill, kept outside the repository. `export-svg.mjs` needs Chrome or Chromium.

```bash
# 1. Validate the specification
node "$ARCHIFY_SKILL/bin/archify.mjs" validate architecture tooling/diagrams/specs/spoke-overview.architecture.json --quality showcase --json

# 2. Deliver the interactive HTML into a scratch directory
node "$ARCHIFY_SKILL/bin/archify.mjs" deliver architecture tooling/diagrams/specs/spoke-overview.architecture.json /tmp/spoke-overview.html --quality showcase --json

# 3. Export static light/dark SVGs for README and VitePress
ARCHIFY_SKILL="$ARCHIFY_SKILL" ARCHIFY_CHROME="/path/to/chrome" \
  node tooling/diagrams/export-svg.mjs /tmp/spoke-overview.html docs/public/diagrams

# 4. Check the committed set
pnpm docs:diagrams
```

The specs carry no `meta.output`; pass the target path on the command line. The export helper inlines computed presentation styles, resolves the theme background, and keeps the `<title>` / `aria-labelledby` pair so the SVG retains an accessible name.
