import { defineConfig } from 'tsup'

// Three isolated entries:
//   - src/index.ts          -> dist/index.{mjs,js} + .d.{mts,ts}  (browser-safe; no `ws`)
//   - src/node/connect-client.ts -> dist/node/connect-client.{mjs,js} + .d.*  (Node; imports `ws`)
//   - src/noise/index.ts    -> dist/noise/index.{mjs,js} + .d.{mts,ts}  (opt-in Noise XX subpath;
//     imports `@noble/ciphers` + `@noble/curves`, which the default entries never resolve)
// `ws` and other node_modules deps stay external (runtime deps, not bundled).
export default defineConfig({
  entry: ['src/index.ts', 'src/node/connect-client.ts', 'src/noise/index.ts'],
  format: ['cjs', 'esm'],
  dts: true,
  clean: true,
  sourcemap: true,
  treeshake: true,
})
