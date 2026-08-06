import { defineConfig } from 'vitepress'

// Vite resolution: `vitepress@1.6.4` declares `vite: ^5.4.14`, but the root
// `package.json` `pnpm.overrides` pins `vite` to `^6.4.3` to clear the
// dev-server advisories that affected vite 5.4.x (GHSA-fx2h-pf6j-xcff high,
// GHSA-4w7w-66w2-5vf9 / GHSA-v6wh-96g9-6wx3 moderate). vitepress 1.6.4 builds
// cleanly against vite 6.4.x (`pnpm docs:build` green); revisit the override
// when vitepress stable moves to vite 6+ natively. `pnpm audit --prod` is clean
// and CI runs only `vitepress build`, which produces static HTML.

// Base URL: GitHub Pages custom domain `spoke.42ch.dev` serves the site at the
// domain root (project-site github.io URL redirects here). Use `base: '/'`.
// If Pages ever drops the custom domain and returns to
// `https://42ch-dev.github.io/spoke/`, switch back to `base: '/spoke/'`.
const base = '/'

// Locales: English is the root locale (served at `/`), 简体中文 is served under
// `/zh/`. Each locale carries its own `themeConfig` (nav + sidebar); these are
// shallow-merged with the top-level `themeConfig`. With two `locales` entries,
// VitePress renders the locale switch in the nav bar automatically — the
// switch button and dropdown use each locale's `label`. (VitePress 1.6.x has
// no separate `selectText`/`langLabel` switch keys; those exist only for the
// search widgets, so the `label` field is the sole switch affordance here.)

// Diátaxis quadrants: Tutorials → How-to guides → Reference → Explanation.
// The "Maintainers" sidebar item links out to root `CONTRIBUTING.md` — the
// release-cut procedure is maintainer-facing and lives there, not on the
// integrator site.

const enNav = [
  { text: 'Home', link: '/' },
  { text: 'Tutorials', link: '/tutorials/install-and-first-entry' },
  { text: 'How-to', link: '/how-to/implement-adapter' },
  { text: 'Reference', link: '/reference/protocol' },
  { text: 'Explanation', link: '/explanation/concepts' },
  { text: 'Packages', link: '/packages/quick-start' },
  { text: 'Release', link: '/release/versioning' },
]

const enSidebar = [
  {
    text: 'Tutorials',
    collapsed: false,
    items: [
      { text: 'Install & first entry', link: '/tutorials/install-and-first-entry' },
      { text: 'First connect session', link: '/tutorials/first-connect-session' },
    ],
  },
  {
    text: 'How-to guides',
    collapsed: false,
    items: [
      { text: 'Implement an adapter', link: '/how-to/implement-adapter' },
      { text: 'TypeScript client', link: '/how-to/connect-ts-client' },
      { text: 'RemoteAdapter over Transport', link: '/how-to/connect-remote-adapter' },
      { text: 'Route across multiple peers', link: '/how-to/multi-peer-routing' },
      { text: 'Native bindings', link: '/how-to/connect-native-bindings' },
      { text: 'RemoteAdapter from native bindings', link: '/how-to/remote-adapter-native-binding' },
      { text: 'Orchestrate operations', link: '/how-to/orchestrate-ops' },
      { text: 'ToyWorld reference adapter', link: '/how-to/walk-toy-world' },
    ],
  },
  {
    text: 'Reference',
    collapsed: false,
    items: [
      { text: 'Protocol', link: '/reference/protocol' },
      { text: 'Data model', link: '/reference/data-model' },
      { text: 'Ops wire', link: '/reference/ops' },
      { text: 'Connect', link: '/reference/connect' },
    ],
  },
  {
    text: 'Explanation',
    collapsed: false,
    items: [
      { text: 'Concepts', link: '/explanation/concepts' },
      { text: 'Connect architecture', link: '/explanation/connect' },
      { text: 'Domain profiles', link: '/explanation/domain-profiles' },
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
  {
    text: 'Maintainers',
    collapsed: false,
    items: [
      {
        text: 'CONTRIBUTING.md',
        link: 'https://github.com/42ch-dev/spoke/blob/main/CONTRIBUTING.md',
      },
    ],
  },
]

// 简体中文 nav + sidebar — CN labels follow the docs i18n glossary; link
// targets point into the `/zh/` tree, one twin per EN page. EN wire
// identifiers (KnowledgeEntry, ops, connect …) are kept verbatim per the SSOT
// rule.
const zhNav = [
  { text: '首页', link: '/zh/' },
  { text: '教程', link: '/zh/tutorials/install-and-first-entry' },
  { text: '操作指南', link: '/zh/how-to/implement-adapter' },
  { text: '参考', link: '/zh/reference/protocol' },
  { text: '讲解', link: '/zh/explanation/concepts' },
  { text: '软件包', link: '/zh/packages/quick-start' },
  { text: '发布', link: '/zh/release/versioning' },
]

const zhSidebar = [
  {
    text: '教程',
    collapsed: false,
    items: [
      { text: '安装与第一条 KnowledgeEntry', link: '/zh/tutorials/install-and-first-entry' },
      { text: '首个 connect 会话', link: '/zh/tutorials/first-connect-session' },
    ],
  },
  {
    text: '操作指南',
    collapsed: false,
    items: [
      { text: '实现 Adapter', link: '/zh/how-to/implement-adapter' },
      { text: 'TypeScript 客户端', link: '/zh/how-to/connect-ts-client' },
      { text: '通过 Transport 使用 RemoteAdapter', link: '/zh/how-to/connect-remote-adapter' },
      { text: '跨多个对等节点路由', link: '/zh/how-to/multi-peer-routing' },
      { text: '原生绑定', link: '/zh/how-to/connect-native-bindings' },
      { text: '从原生绑定使用 RemoteAdapter', link: '/zh/how-to/remote-adapter-native-binding' },
      { text: '编排操作', link: '/zh/how-to/orchestrate-ops' },
      { text: 'ToyWorld 参考适配器', link: '/zh/how-to/walk-toy-world' },
    ],
  },
  {
    text: '参考',
    collapsed: false,
    items: [
      { text: '协议', link: '/zh/reference/protocol' },
      { text: '数据模型', link: '/zh/reference/data-model' },
      { text: '操作线上（Ops wire）', link: '/zh/reference/ops' },
      { text: 'connect', link: '/zh/reference/connect' },
    ],
  },
  {
    text: '讲解',
    collapsed: false,
    items: [
      { text: '核心概念', link: '/zh/explanation/concepts' },
      { text: 'Connect 架构', link: '/zh/explanation/connect' },
      { text: '领域画像', link: '/zh/explanation/domain-profiles' },
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
  {
    text: '维护者',
    collapsed: false,
    items: [
      {
        text: 'CONTRIBUTING.md',
        link: 'https://github.com/42ch-dev/spoke/blob/main/CONTRIBUTING.md',
      },
    ],
  },
]

export default defineConfig({
  title: 'SPOKE Protocol',
  description:
    'SPOKE — Standardized Programmable Ontology Knowledge Engine. Integrator documentation: tutorials, how-to guides, reference, and explanation.',
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
