import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "vitest/config";

const REPO_ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "../../..");

export default defineConfig({
  resolve: {
    alias: {
      // Alias to src/ so tests run without a prior dist build of the
      // workspace deps (mirrors packages/spoke-connect-ts/vitest.config.ts;
      // CI does not build @42ch/spoke-connect dist).
      "@42ch/spoke-schemas": join(
        REPO_ROOT,
        "packages/spoke-schemas/src/index.ts",
      ),
      // Order matters: the /remote subpath MUST come before the bare name,
      // otherwise vite prefix-matches the shorter key first.
      "@42ch/spoke-connect/remote": join(
        REPO_ROOT,
        "packages/spoke-connect-ts/src/remote/index.ts",
      ),
      "@42ch/spoke-connect": join(
        REPO_ROOT,
        "packages/spoke-connect-ts/src/index.ts",
      ),
      "@42ch/spoke-operations": join(
        REPO_ROOT,
        "packages/spoke-operations/src/index.ts",
      ),
      // The demo server is a devDependency of the client (e2e only) — alias
      // to its source entry so the e2e needs no prior demo build either.
      "@42ch/spoke-demo-server": join(
        REPO_ROOT,
        "examples/connect-demo/server/src/index.ts",
      ),
    },
  },
  test: {
    include: ["tests/**/*.test.ts"],
  },
});
