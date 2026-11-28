import { defineConfig } from 'tsup'

// `ws` and other node_modules deps stay external (runtime deps, not bundled).
// `canonicalize` is the one exception: it ships ESM-only (no `require`
// condition in its exports map), so an external CJS require() would throw
// ERR_PACKAGE_PATH_NOT_EXPORTED for CJS consumers. It is a tiny dependency-
// free file, so bundling it into both formats keeps the dual package usable.
export default defineConfig({
  entry: [
    'src/index.ts',
    'src/node/connect-client.ts',
    'src/noise/index.ts',
    'src/remote/index.ts',
  ],
  format: ['cjs', 'esm'],
  dts: true,
  clean: true,
  sourcemap: true,
  treeshake: true,
  noExternal: ['canonicalize'],
})
