import { defineConfig } from 'vitepress'

// Vite resolution: `vitepress@1.6.4` declares `vite: ^5.4.14`, but the root
// `package.json` `pnpm.overrides` pins `vite` to `^6.4.3` to clear the
// dev-server advisories that affected vite 5.4.x (GHSA-fx2h-pf6j-xcff high,
// GHSA-4w7w-66w2-5vf9 / GHSA-v6wh-96g9-6wx3 moderate). vitepress 1.6.4 builds
// cleanly against vite 6.4.x (`pnpm docs:build` green); revisit the override
// when vitepress stable moves to vite 6+ natively. `pnpm audit --prod` is clean
// and CI runs only `vitepress build`, which produces static HTML.

// Base URL: this site is published as a GitHub Pages *project site* at
// https://42ch-dev.github.io/spoke/ (repository 42ch-dev/spoke). Keep
// `base: '/spoke/'` unless Pages moves to a custom domain or the org root
// site (then use `base: '/'`).
const base = '/spoke/'

// Locales: English is the root locale (served at `/`), 简体中文 is served under
// `/zh/`. Each locale carries its own `themeConfig` (nav + sidebar); these are
// shallow-merged with the top-level `themeConfig`. With two `locales` entries,
// VitePress renders the locale switch in the nav bar automatically — the
// switch button and dropdown use each locale's `label`. (VitePress 1.6.x has
// no separate `selectText`/`langLabel` switch keys; those exist only for the
// search widgets, so the `label` field is the sole switch affordance here.)

// English nav + sidebar (mirrors the former EN-only `themeConfig`).
const enNav = [
  { text: 'Home', link: '/' },
  { text: 'Protocol', link: '/guide/protocol' },
  { text: 'Guides', link: '/guide/concepts' },
  { text: 'Connect', link: '/connect/overview' },
  { text: 'Packages', link: '/packages/quick-start' },
  { text: 'Release', link: '/release/versioning' },
]

const enSidebar = [
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
]

// 简体中文 nav + sidebar — CN labels follow the docs i18n glossary; link
// targets point into the `/zh/` tree, one twin per EN page (same 17-page
// scope). EN wire identifiers (KnowledgeEntry, ops, connect …) are kept
// verbatim per the SSOT rule.
const zhNav = [
  { text: '首页', link: '/zh/' },
  { text: '协议', link: '/zh/guide/protocol' },
  { text: '指南', link: '/zh/guide/concepts' },
  { text: '连接', link: '/zh/connect/overview' },
  { text: '软件包', link: '/zh/packages/quick-start' },
  { text: '发布', link: '/zh/release/versioning' },
]

const zhSidebar = [
  {
    text: '指南',
    collapsed: false,
    items: [
      { text: '核心概念', link: '/zh/guide/concepts' },
      { text: '协议总览', link: '/zh/guide/protocol' },
      { text: '分层与能力', link: '/zh/guide/layers' },
      { text: '数据模型', link: '/zh/guide/data-model' },
      { text: '操作线上信封（Ops wire）', link: '/zh/guide/ops-wire' },
      { text: '操作库', link: '/zh/guide/operations-library' },
      { text: '扩展与模块', link: '/zh/guide/extensions-modules' },
    ],
  },
  {
    text: '领域画像',
    collapsed: false,
    items: [
      { text: '叙事结构', link: '/zh/profiles/narrative-structure' },
      { text: '世界观激活（lore activation）', link: '/zh/profiles/lore-activation' },
      { text: '知识包', link: '/zh/profiles/knowledge-pack' },
      { text: 'assemble 模块配方', link: '/zh/profiles/assemble-recipes' },
    ],
  },
  {
    text: '连接',
    collapsed: false,
    items: [
      { text: '总览', link: '/zh/connect/overview' },
      { text: 'TypeScript 路线', link: '/zh/connect/ts-route' },
      { text: '原生绑定', link: '/zh/connect/bindings' },
    ],
  },
  {
    text: '软件包',
    collapsed: false,
    items: [{ text: '软件包快速开始', link: '/zh/packages/quick-start' }],
  },
  {
    text: '发布',
    collapsed: false,
    items: [{ text: '版本与发布', link: '/zh/release/versioning' }],
  },
]

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
  locales: {
    root: {
      label: 'English',
      lang: 'en',
      themeConfig: {
        nav: enNav,
        sidebar: enSidebar,
      },
    },
    zh: {
      label: '简体中文',
      lang: 'zh-CN',
      themeConfig: {
        nav: zhNav,
        sidebar: zhSidebar,
      },
    },
  },
})
