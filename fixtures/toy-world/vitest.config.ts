import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "vitest/config";

const REPO_ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "../..");

export default defineConfig({
  resolve: {
    alias: {
      "@42ch/spoke-operations": join(
        REPO_ROOT,
        "packages/spoke-operations/src/index.ts",
      ),
    },
  },
  test: {
    include: ["tests/**/*.test.ts"],
  },
});
