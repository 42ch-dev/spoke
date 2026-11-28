import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "vitest/config";

const REPO_ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "../..");

export default defineConfig({
  resolve: {
    alias: {
      "@42ch/spoke-schemas": join(
        REPO_ROOT,
        "packages/spoke-schemas/src/index.ts",
      ),
      // Order matters: the /node, /noise and /remote subpaths MUST come
      // before the bare name, otherwise vite prefix-matches the shorter key
      // first.
      "@42ch/spoke-connect/noise": join(
        REPO_ROOT,
        "packages/spoke-connect-ts/src/noise/index.ts",
      ),
      "@42ch/spoke-connect/node": join(
        REPO_ROOT,
        "packages/spoke-connect-ts/src/node/connect-client.ts",
      ),
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
      "@42ch/spoke-fixture-toy-world": join(
        REPO_ROOT,
        "fixtures/toy-world/src/adapter/index.ts",
      ),
    },
  },
  test: {
    include: ["tests/**/*.test.ts"],
  },
});
