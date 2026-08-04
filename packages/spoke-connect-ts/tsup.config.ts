import { defineConfig } from 'tsup'

// Three isolated entries:
//   - src/index.ts          -> dist/index.{mjs,js} + .d.{mts,ts}  (browser-safe; no `ws`)
//   - src/node/connect-client.ts -> dist/node/connect-client.{mjs,js} + .d.*  (Node; imports `ws`)
//   - src/noise/index.ts    -> dist/noise/index.{mjs,js} + .d.{mts,ts}  (opt-in Noise XX subpath;
//     imports `@noble/ciphers` + `@noble/curves`, which the default entries never resolve)
// `ws` and other node_modules deps stay external (runtime deps, not bundled).
// `canonicalize` is the one exception: it ships ESM-only (no `require`
// condition in its exports map), so an external CJS require() would throw
// ERR_PACKAGE_PATH_NOT_EXPORTED for CJS consumers. It is a tiny dependency-
// free file, so bundling it into both formats keeps the dual package usable.
export default defineConfig({
  entry: ['src/index.ts', 'src/node/connect-client.ts', 'src/noise/index.ts'],
  format: ['cjs', 'esm'],
  dts: true,
  clean: true,
  sourcemap: true,
  treeshake: true,
  noExternal: ['canonicalize'],
})
