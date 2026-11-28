import { build } from "esbuild";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const PKG_ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "../..");

interface Trace {
  files: Set<string>;
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
    expect(hasFileSegment(trace, "noise")).toBe(true);
    expect(hasFileSegment(trace, "core")).toBe(false);
  });
});
