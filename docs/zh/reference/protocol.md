---
title: 协议参考
---

# 协议参考（Protocol reference）

SPOKE 是面向叙事产品的共享**线上方言（wire dialect）**：一套用于知识数据与操作的 JSON Schema 契约，使产品在共同的协议面上交换 KnowledgeEntry 数据与操作。协议横跨三列，外加一个可选加入的 connect 信封族。

## 三列模型

| 列 | 内容 |
|----|------|
| **数据线上（Data wire）** | 八个持久对象：KnowledgeEntry、Relation、SourceAnchor、Finding、AssemblePacket、HostCapabilityManifest、Rule、TimelineEvent（[`schemas/data/`](https://github.com/42ch-dev/spoke/tree/main/schemas/data)），共享定义在 [`schemas/common/`](https://github.com/42ch-dev/spoke/tree/main/schemas/common) |
| **操作线上（Ops wire）** | 五个基线操作族 —— upsert、extract→promote、relate、check、assemble —— 以传输无关的请求/响应信封承载（[`schemas/ops/`](https://github.com/42ch-dev/spoke/tree/main/schemas/ops)），`l2-computable` 下另有可选 `project` / `compute` |
| **操作库（Operations library）** | 在生成的线上类型之上的手写行为层：纯函数生命周期辅助、按能力切片的 adapter ports 与注入式编排（TypeScript `@42ch/spoke-operations`；Rust `spoke-operations`，锁步 SemVer） |

## connect 信封族（可选加入）

六个交互信封（[`schemas/connect/`](https://github.com/42ch-dev/spoke/tree/main/schemas/connect)，能力标志 `spoke-connect`）增加跨进程交互：以 `$ref` 内嵌 `HostCapabilityManifest` 的签名握手（hello）、会话上下文、将既有操作信封作为不透明载荷包装的 invoke 请求/响应，以及鉴权挑战/响应。该族是增量的 —— 基线合规与基线 schema 保持不变。见 [connect 参考](/zh/reference/connect)。

## Schema 清单与代码生成姿态

线上清单为 **30 个已提交的 `*.schema.json`**：2 common + 8 data + 14 ops + 6 connect 信封。`schemas/` 是唯一手写线上真源；生成的 TypeScript（`@42ch/spoke-schemas`）与 Rust（`spoke-schemas`）输出提交入库并镜像 schema 树。`pnpm run verify-codegen` 在生成树偏离 `schemas/` 时令构建失败；schema 变更与重新生成输出落在同一提交。

## 扩展契约

每个持久对象携带必填的 `extensions.<namespace>` 字段袋；核心字段保持封闭（`additionalProperties: false`）。

| 字段袋 | 形状 | 角色 |
|--------|------|------|
| `extensions.<namespace>` | 每个持久数据对象上的必填 `ExtensionMap`；namespace 键为产品自选 id，匹配 `^[a-z][a-z0-9_-]*$`；值为不透明 JSON 对象 | 单一产品的私有袋。adapter 在往返中保留未知 namespace 与键 |
| `modules.*` | KnowledgeEntry 与 AssemblePacket 上的可选 `ModuleMap`（能力标志 `narrative-modules`）；键为功能方言 id（`activation`、`placement`、`activation_trace` 等）；值为结构化 JSON，内部形状由手册定义 | 叙事主机共享的跨产品功能方言。未知模块键原样往返 |

放置规则：**跨产品功能方言**用 `modules.*`；**产品数据**用 `extensions.<product>`。在 `HostCapabilityManifest` 上，`extensions` 携带部署元数据 —— roles、capabilities 与 namespace 所有权是核心 manifest 字段。

## 能力标志

| 标志 | 增加什么 |
|------|----------|
| `spoke-baseline` | 经五个 ops 线上族、`HostCapabilityManifest` + 基线 `HostManifestPort`、以及共享 `Scope` / `error-envelope` 定义获得 L0–L8 语义。基线合规可独立成立；可选标志是增量的 |
| `l2-computable` | KnowledgeEntry 上的 `body.state` / `body.computable`、`TimelineEvent.computable_logs`，以及 `project` / `compute` ops |
| `l5-fork` | TimelineEvent 上的 `fork_id` / `parent_fork_id` 分支元数据与 `Scope.fork_id` 过滤 |
| `narrative-modules` | KnowledgeEntry + AssemblePacket 上的可选 `modules`（`ModuleMap`）字段袋 |
| `spoke-connect` | 可选交互信封族；讲该协议的主机在 `HostCapabilityManifest.capabilities` 中列出该标志 |

## 仓库布局

`schemas/`（SSOT）· `tooling/codegen/` · `packages/spoke-schemas` + `packages/spoke-operations`（TypeScript）· `crates/spoke-schemas` + `crates/spoke-operations`（Rust）· `fixtures/toy-world/`（一致性样例与参考 adapter）。

## 相关页面

- [数据模型参考](/zh/reference/data-model) —— 八个持久对象的字段表。
- [操作线上参考（Ops wire）](/zh/reference/ops) —— 请求/响应信封、`Scope`、`ErrorEnvelope`。
- [connect 参考](/zh/reference/connect) —— 可选信封族。
- [核心概念](/zh/explanation/concepts) —— 九层模型与能力标志如何映射。
- [`schemas/README.md`](https://github.com/42ch-dev/spoke/blob/main/schemas/README.md) —— schema 文件清单。
