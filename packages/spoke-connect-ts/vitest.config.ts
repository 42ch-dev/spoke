import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "vitest/config";

const REPO_ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "../..");

export default defineConfig({
  resolve: {
    alias: {
      // Alias to schemas src/ so tests run without a prior `build:schema`
      // (same pattern as the fixture → operations alias; AD-P0-1).
      "@42ch/spoke-schemas": join(
        REPO_ROOT,
        "packages/spoke-schemas/src/index.ts",
      ),
      // Self-import aliases so tests run from src/ without a prior build
      // (the package exports point to dist/ for published consumers).
      "@42ch/spoke-connect": join(
        REPO_ROOT,
        "packages/spoke-connect-ts/src/index.ts",
      ),
      "@42ch/spoke-connect/node": join(
        REPO_ROOT,
        "packages/spoke-connect-ts/src/node/connect-client.ts",
      ),
    },
  },
  test: {
    include: ["tests/**/*.test.ts"],
  },
});
