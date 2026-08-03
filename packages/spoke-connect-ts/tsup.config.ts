import { defineConfig } from 'tsup'

// Two isolated entries:
//   - src/index.ts          -> dist/index.{mjs,js} + .d.{mts,ts}  (browser-safe; no `ws`)
//   - src/node/connect-client.ts -> dist/node/connect-client.{mjs,js} + .d.*  (Node; imports `ws`)
// `ws` and other node_modules deps stay external (runtime deps, not bundled).
export default defineConfig({
  entry: ['src/index.ts', 'src/node/connect-client.ts'],
  format: ['cjs', 'esm'],
  dts: true,
  clean: true,
  sourcemap: true,
  treeshake: true,
})
