/**
 * Bundle-isolation gate for the opt-in `./noise` subpath (plan "Global
 * Constraints" — no default-dep drift; noise-subpath rationale §Bundle
 * isolation guarantee).
 *
 * A dependency trace over the SOURCE entries (esbuild metafile): CI runs
 * `test:connect-ts` without building `@42ch/spoke-connect`, so the gate
 * must not depend on dist/. Every bare import is externalized, so the
 * metafile records the package names the graph reaches without needing
 * node_modules resolution of the workspace's published dist.
 *
 *   - `@42ch/spoke-connect` (default `.`): must NOT resolve
 *     `@noble/ciphers` / `@noble/curves` (Noise-only deps) and must not
 *     contain any `src/noise/**` module.
 *   - `@42ch/spoke-connect/noise`: must resolve both Noise-only deps, and
 *     must not reach `src/core/**` (the session-core parity surface) — it
 *     may reuse the shared src-root helpers (`src/crypto.ts`,
 *     `src/identity.ts`) that are already in the default bundle.
 *
 * The default-graph assertions have a positive control (the default deps
 * `@noble/ed25519` / `@noble/hashes` MUST appear) so the tracer cannot
 * pass vacuously.
 */
import { build } from "esbuild";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const PKG_ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "../..");

interface Trace {
  /** Every bundled (non-external) module path in the entry's graph. */
  files: Set<string>;
  /** Bare specifiers of every external import in the entry's graph. */
  externals: Set<string>;
}

async function traceDeps(entry: string): Promise<Trace> {
  const result = await build({
    entryPoints: [join(PKG_ROOT, "src", entry)],
    bundle: true,
    write: false,
    metafile: true,
    format: "esm",
    platform: "neutral",
    // Every node_modules package stays external: we only need the import
    // graph, not bundled output.
    packages: "external",
  });
  if (result.metafile === undefined) {
    throw new Error("esbuild metafile missing — trace failed");
  }
  const files = new Set(Object.keys(result.metafile.inputs));
  const externals = new Set<string>();
  for (const file of files) {
    for (const imp of result.metafile.inputs[file].imports) {
      if (imp.external) externals.add(imp.path);
    }
  }
  return { files, externals };
}

const hasExternalPrefix = (trace: Trace, prefix: string): boolean =>
  [...trace.externals].some((specifier) => specifier.startsWith(prefix));

const hasFileSegment = (trace: Trace, segment: string): boolean =>
  [...trace.files].some((file) => file.includes(`/${segment}/`));

describe("bundle isolation: `./noise` subpath vs default `.` entry", () => {
  it("default entry resolves the thin default deps (tracer positive control)", async () => {
    const trace = await traceDeps("index.ts");
    expect(trace.files.size).toBeGreaterThan(0);
    expect(hasExternalPrefix(trace, "@noble/ed25519")).toBe(true);
    expect(hasExternalPrefix(trace, "@noble/hashes")).toBe(true);
  });

  it("default entry never resolves Noise-only deps nor any src/noise/** module", async () => {
    const trace = await traceDeps("index.ts");
    expect(hasExternalPrefix(trace, "@noble/ciphers")).toBe(false);
    expect(hasExternalPrefix(trace, "@noble/curves")).toBe(false);
    expect(hasFileSegment(trace, "noise")).toBe(false);
  });

  it("noise entry resolves both Noise-only deps and its own modules", async () => {
    const trace = await traceDeps("noise/index.ts");
    expect(hasExternalPrefix(trace, "@noble/ciphers")).toBe(true);
    expect(hasExternalPrefix(trace, "@noble/curves")).toBe(true);
    // The subpath shares the src-root helpers already in the default
    // bundle (allowed by plan Global Constraints), but never reaches
    // the session-core parity surface.
    expect(hasFileSegment(trace, "noise")).toBe(true);
    expect(hasFileSegment(trace, "core")).toBe(false);
  });
});
