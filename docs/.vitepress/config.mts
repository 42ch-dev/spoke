import { defineConfig } from 'vitepress'

// Dependency risk disposition (accepted; revisit on the vitepress 2.x / vite 6
// upgrade): the `vitepress@1.6.4` devDependency pins `vite@5.4.21`, which
// carries GHSA-fx2h-pf6j-xcff (high) plus GHSA-4w7w-66w2-5vf9 and
// GHSA-v6wh-96g9-6wx3 (moderate) — all dev-server surfaces, with no in-range
// remediation (patches land in vite 6.4.x, which vitepress 1.6.x excludes).
// `pnpm audit --prod` is clean and CI runs only `vitepress build`, which
// produces static HTML.

// Base URL: this site is published as a GitHub Pages *project site* at
// https://42ch-dev.github.io/spoke/ (repository 42ch-dev/spoke). Keep
// `base: '/spoke/'` unless Pages moves to a custom domain or the org root
// site (then use `base: '/'`).
const base = '/spoke/'

export default defineConfig({
  title: 'SPOKE Protocol',
  description:
    'SPOKE — Standardized Programmable Ontology Knowledge Engine. Integrator documentation: protocol, Domain Profiles, connect, packages.',
  base,
  vite: {
    build: {
      // simplify: esbuild >= 0.25 (forced repo-wide by the root `esbuild` security
      // override) regresses on destructuring when Vite's default 'modules' target
      // expands to browser-version targets (chrome87/edge88/...). An explicit
      // ECMAScript-version target (es2020 — Vite's prior browser baseline) avoids
      // the browser list entirely. Revisit if Vite is upgraded to a version
      // compatible with esbuild >= 0.25.
      target: 'es2020',
    },
    optimizeDeps: {
      // Same esbuild regression as above: the dep optimizer hardcodes Vite's
      // browser-version target list, so pin it to a pure ECMAScript target
      // (es2020 — Vite's prior browser baseline).
      esbuildOptions: {
        target: 'es2020',
      },
    },
  },
  themeConfig: {
    nav: [
      { text: 'Home', link: '/' },
      { text: 'Protocol', link: '/guide/protocol' },
      { text: 'Guides', link: '/guide/concepts' },
      { text: 'Connect', link: '/connect/overview' },
      { text: 'Packages', link: '/packages/quick-start' },
      { text: 'Release', link: '/release/versioning' },
    ],
    sidebar: [
      {
        text: 'Guides',
        collapsed: false,
        items: [
          { text: 'Concepts', link: '/guide/concepts' },
          { text: 'Protocol umbrella', link: '/guide/protocol' },
          { text: 'Layers & capabilities', link: '/guide/layers' },
          { text: 'Data model', link: '/guide/data-model' },
          { text: 'Ops wire', link: '/guide/ops-wire' },
          { text: 'Operations library', link: '/guide/operations-library' },
          { text: 'Extensions & modules', link: '/guide/extensions-modules' },
        ],
      },
      {
        text: 'Domain Profiles',
        collapsed: false,
        items: [
          { text: 'Narrative structure', link: '/profiles/narrative-structure' },
          { text: 'Lore activation', link: '/profiles/lore-activation' },
          { text: 'Knowledge pack', link: '/profiles/knowledge-pack' },
          { text: 'Assemble module recipes', link: '/profiles/assemble-recipes' },
        ],
      },
      {
        text: 'Connect',
        collapsed: false,
        items: [
          { text: 'Overview', link: '/connect/overview' },
          { text: 'TypeScript route', link: '/connect/ts-route' },
          { text: 'Native bindings', link: '/connect/bindings' },
        ],
      },
      {
        text: 'Packages',
        collapsed: false,
        items: [{ text: 'Package quick-start', link: '/packages/quick-start' }],
      },
      {
        text: 'Release',
        collapsed: false,
        items: [{ text: 'Version & release', link: '/release/versioning' }],
      },
    ],
  },
})
