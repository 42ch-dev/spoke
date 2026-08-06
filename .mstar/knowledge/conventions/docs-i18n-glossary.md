---
module: docs
date: 2026-08-03
problem_type: convention
category: conventions
severity: medium
applies_when:
  - "authoring or reviewing CN twins of integrator docs"
  - "adding VitePress zh locale nav / sidebar labels"
  - "resolving EN/CN terminology drift against README_CN or CONCEPTS"
tags: [i18n, glossary, cn, vitepress, terminology, twin-docs]
---

# Integrator docs i18n — CN terminology glossary

## Context

Integrator-facing docs ship EN root pages plus zh CN twins (VitePress
`docs/` + `docs/zh/`, and root `README.md` / `README_CN.md`). Wire identifiers
stay English; CN prose glosses them. Without a single terminology sheet,
translators invent alternate glosses (`事件` alone for TimelineEvent,
`配置文件` for Domain Profile) that break dual-concern clarity and twin
outline consistency. This glossary is the durable alignment reference for
CN docs work.

## Guidance

### Authority order

1. **`README_CN.md`** — established CN twin vocabulary for this repo; keep
   its spellings verbatim when they already cover a term.
2. **`CONCEPTS.md`** — English SSOT for wire-term definitions; CN prose
   glosses the English term, it does not replace it.
3. **This glossary** — resolves ambiguity between the two above; where this
   glossary differs from an existing `README_CN.md` spelling, fix the
   glossary.
4. **Translator judgment** for running prose, following the twin outline of
   `README.md` / `README_CN.md` (and the matching VitePress page pair).

### Wire terms (keep the English term; CN gloss on first use per page)

| EN term | CN usage | Fixed by | Note |
|---------|----------|----------|------|
| KnowledgeEntry | 知识条目（叙事知识单元）；首用可写作"KnowledgeEntry（知识条目）" | README_CN 核心概念表 | Wire name stays `KnowledgeEntry` in code/identifiers |
| TimelineEvent | 时间轴事件（when 轴第一类时间对象） | README_CN | Prefer the full gloss; keep distinct from `entry_type: "event"` (dual-concern) |
| Relation | 关系（有向边） | README_CN | |
| SourceAnchor | 溯源指针 | README_CN | |
| Finding | 检查器输出 | README_CN | Distinct from KnowledgeEntry `body` |
| Rule | 检查规则（`check` 的声明式约束输入） | README_CN | Distinct from `entry_type: "rule"` ontology tag |
| Scope | 查询范围（check/assemble 共享选择器） | CONCEPTS §Scope | |
| AssemblePacket | 上下文组装载荷（供下游 LLM 提示的精简条目） | README_CN | |
| HostCapabilityManifest | 主机能力清单（主机角色、能力与所拥有的 `namespaces[]`） | README_CN | |
| Extensions | 扩展字段袋（产品专属，`extensions.<namespace>`） | README_CN | |
| Modules | 模块字段袋（可选跨产品功能方言，按能力启用） | README_CN | |
| Domain Profile | 领域画像；正文保留英文"Domain Profile（领域画像）" | README_CN"Domain Profile 手册" | Product-side term; avoid translating as 配置文件 |
| peer_id | 对等节点标识（保留英文 `peer_id`） | CONCEPTS §peer_id | |
| host_id | 主机标识（保留英文 `host_id`） | CONCEPTS §peer_id | Distinct trust root from `peer_id` |
| capability flag | 能力标志；可选能力按需启用 | README_CN | Capability **names** stay English: `spoke-baseline`, `l2-computable`, `l5-fork`, `narrative-modules`, `spoke-connect` |
| connect | 连接层（opt-in 交互信封族）；正文保留英文 "connect" | README_CN / CONCEPTS §spoke-connect | Site Connect nav title → 连接 (see nav table) |
| capability token | 能力令牌（短期、按能力授权的授权证明，Ed25519 签名） | CONCEPTS §capability token | First use: "capability token（能力令牌）" |
| trusted issuers | 受信任签发方 | CONCEPTS §capability token | |
| session (connect) | 会话（跨进程调用上下文：`session_id`、序列与 `request_id` 关联） | CONCEPTS §Session (connect) | Homonym with `l2-computable` Session — add a qualifier by context on CN pages |
| hello / ConnectHello | 握手（已签名清单交换） | CONCEPTS §Connect envelope family | |
| envelope | 信封（交互消息信封） | CONCEPTS §Connect envelope family | |
| wire | 线上（线上契约 / 线上传输形状） | README_CN"线上契约" | |
| normative / spec | 规范；规范说明（SSOT 链接目标） | README_CN"规范" | |
| SSOT | 保留英文 SSOT（单一事实来源） | README_CN | |
| dual-concern | 双重关注（同一概念的两类线形：本体标签 vs 时间对象） | CONCEPTS §Dual-concern | First appearance: keep English alongside the gloss |
| lockstep SemVer | 锁步 SemVer | README_CN | |
| adapter ports | 保留英文 adapter ports（注入式读写面） | README_CN | |
| orchestration | 编排（`orchestrate*` / `orchestrate_*` 序列） | README_CN | |
| op / ops | 操作（`upsert`、`promote`、`relate`、`check`、`assemble` 保留英文） | README_CN | |
| packages | 软件包 | README_CN | |
| quick start | 快速开始 | README_CN | |
| release / versioning | 发布 / 版本管理 | README_CN | |
| locale switch | 语言切换（VitePress `localeLink`） | — | UI chrome: "中文 / English" |
| language-native client | 语言原生客户端 | connect 家族 how-to/explanation 页 | Freeze to prevent drift from 原生客户端 / 语言客户端 |
| Transport | 传输接口（保留英文 `Transport`） | connect how-to 页（zh 既有用法） | Consumer-implemented seam; first use "`Transport`（传输接口）", then bare English |
| RemoteAdapter | 保留英文 RemoteAdapter；首用可写"RemoteAdapter（远程适配器）" | connect how-to 页（zh 既有用法） | Wire-surface term, same pattern as Domain Profile |
| multi-peer router | 多对等节点路由器 | connect how-to/explanation 页 | 页面标题 / 导航 / index 卡片 "Route across multiple peers" → **跨多个对等节点路由**（freeze **DONE**：zh sidebar、页面标题与 index 卡片已全部对齐） |
| envelope authentication | 信封认证（逐信封签名） | connect reference / explanation 页 | New with protocol_version 2; 协议版本 2 下 post-hello 信封必填签名 |
| loopback | 回环（测试用途） | connect how-to 页 | Test-only qualifier; 回环对 loopback pair |
| FfiError / TransportError | 保留英文 | connect FFI 页 | Error-surface identifiers, never translated |
| protocol_version | 保留英文 `protocol_version`；正文可写协议版本 2 | connect reference 页 | Distinguish from data `schema_version` 版本号 |

### Nav / sidebar / locale labels (VitePress `zh`)

| EN | CN |
|----|-----|
| Home | 首页 |
| Protocol | 协议 |
| Guides | 指南 |
| Connect | 连接 |
| Packages | 软件包 |
| Release | 发布 |
| Domain Profiles (sidebar group) | 领域画像 |
| Concepts | 核心概念 |
| Protocol umbrella | 协议总览 |
| Layers & capabilities | 分层与能力 |
| Data model | 数据模型 |
| Ops wire | 操作线上信封（Ops wire） |
| Operations library | 操作库 |
| Extensions & modules | 扩展与模块 |
| Narrative structure | 叙事结构 |
| Lore activation | 世界观激活（lore activation） |
| Knowledge pack | 知识包 |
| Assemble module recipes | assemble 模块配方 |
| Overview | 总览 |
| TypeScript route | TypeScript 路线 |
| Native bindings | 原生绑定 |
| Package quick-start | 软件包快速开始 |
| Version & release | 版本与发布 |
| Page titles on CN twins | Match the EN page one-to-one; keep English wire terms with CN glosses |
| Route across multiple peers (zh sidebar) | 跨多个对等节点路由（与页面标题一致；2026-08 冻结） |

### SSOT and twin rules

- `.mstar/specs/`, `CONCEPTS.md`, and root `README.md` prose stay **English-only**.
- CN docs pages **summarize + link** to the English normative sources
  (same pattern as `README.md` / `README_CN.md` twins); specs are never
  translated body-for-body.
- Wire identifiers, code spans, JSON field names, and file paths stay
  verbatim English (never transliterate `entry_id`, `peer_id`, `modules.*`).
- CN pages state current capability affirmatively (root `AGENTS.md`
  human-readable docs rule).
- Terminology consistency beats elegant variation: one term → one CN gloss
  per page set; first-use pattern `EN（CN）`, then bare EN term.

### Page inventory pattern

When the docs site grows, keep EN nav/sidebar and CN twins in lockstep:
every EN page in the published set gets a CN twin, and no orphan CN page
exists outside that set. Current site layout lives under `docs/` and
`docs/zh/`; re-count against the nav config when adding pages.

The 1:1 twin mapping is enforced in CI: `tooling/docs/twin-parity.mjs` (run
by the docs workflow on every PR touching `docs/**` or `tooling/docs/**`)
fails the build when a page lands in one locale without its twin in the
other. Pages that legitimately exist in only one locale are listed in the
script's `localeSpecific` allow-list as docs-relative paths.

## Why This Matters

Twin docs fail quietly when glosses drift: dual-concern terms collapse,
capability names get half-translated, and nav labels stop matching the EN
outline. A single glossary anchored on `README_CN.md` + `CONCEPTS.md` keeps
future CN work mechanical and reviewable.

## When to Apply

- Writing or reviewing any `docs/zh/**` page or root `README_CN.md` section.
- Adding a VitePress zh nav/sidebar entry.
- Introducing a new wire or connect term that needs a first-use CN gloss.

## See also

- [`consumer-readme-twin.md`](../architecture-patterns/consumer-readme-twin.md) — EN/CN twin structure and audience rules for human READMEs.
- [`integrator-docs-site-ssot-links.md`](../architecture-patterns/integrator-docs-site-ssot-links.md) — docs site SSOT-link policy (summarize + blob links).
- Root `CONCEPTS.md` — English vocabulary SSOT.
- Root `README_CN.md` — established CN twin spellings.
