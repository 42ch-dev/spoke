---
title: 协议参考
---

# 协议参考（Protocol reference）

SPOKE 是面向叙事产品的共享**线上方言（wire dialect）**：一套用于知识数据与操作的 JSON Schema 契约，使产品在共同的协议面上交换 KnowledgeEntry 数据与操作。协议横跨三列，外加一个可选加入的 connect 信封族。

## 三列模型

| 列 | 内容 |
|----|------|
| **数据线上（Data wire）** | 十一个数据对象：KnowledgeEntry、Relation、SourceAnchor、Finding、AssemblePacket、HostCapabilityManifest、Rule、TimelineEvent、CognitionRecord、MindState、ToolDescriptor（[`schemas/data/`](https://github.com/42ch-dev/spoke/tree/main/schemas/data)），共享定义在 [`schemas/common/`](https://github.com/42ch-dev/spoke/tree/main/schemas/common)；可选 `spoke-memory` 族在同一编写规则下增加十三个记忆模型 |
| **操作线上（Ops wire）** | 五个基线操作族 —— upsert、extract→promote、relate、check、assemble —— 以传输无关的请求/响应信封承载（[`schemas/ops/`](https://github.com/42ch-dev/spoke/tree/main/schemas/ops)），`l2-computable` 下另有可选 `project` / `compute`，`ke-extraction` 下另有可选 `extract`，以及 33 个操作的 `spoke-memory` 族 |
| **操作库（Operations library）** | 在生成的线上类型之上的手写行为层：纯函数生命周期辅助、按能力切片的 adapter ports 与注入式编排（TypeScript `@42ch/spoke-operations`；Rust `spoke-operations`，锁步 SemVer） |

## connect 信封族（可选加入）

六个交互信封（[`schemas/connect/`](https://github.com/42ch-dev/spoke/tree/main/schemas/connect)，能力标志 `spoke-connect`）增加跨进程交互：以 `$ref` 内嵌 `HostCapabilityManifest` 的签名握手（hello）、会话上下文、将既有操作信封作为不透明载荷包装的 invoke 请求/响应，以及鉴权挑战/响应。该族是增量的 —— 基线合规与基线 schema 保持不变。见 [connect 参考](/zh/reference/connect)。

## Schema 清单与代码生成姿态

线上清单为 **114 个已提交的 `*.schema.json`**：2 common + 24 data + 82 ops + 6 connect 信封。可选 `spoke-memory` 族贡献其中 79 个（13 个数据模型 + 33 组请求/响应配对）。基线集成方使用其中的 **21** 个 schema 文件（2 common + 9 data + 10 基线 ops）：九个基线数据 schema 包含 `ToolDescriptor`，而 `CognitionRecord` 与 `MindState` 是两个可选的非记忆数据文件。`schemas/` 是唯一手写线上真源；生成的 TypeScript（`@42ch/spoke-schemas`）与 Rust（`spoke-schemas`）输出提交入库并镜像 schema 树。`pnpm run verify-codegen` 在生成树偏离 `schemas/` 时令构建失败；schema 变更与重新生成输出落在同一提交。

## 扩展契约

每个持久对象携带必填的 `extensions.<namespace>` 字段袋；核心字段保持封闭（`additionalProperties: false`）。

| 字段袋 | 形状 | 角色 |
|--------|------|------|
| `extensions.<namespace>` | 每个持久数据对象上的必填 `ExtensionMap`；namespace 键为产品自选 id，匹配 `^[a-z][a-z0-9_-]*$`；值为不透明 JSON 对象 | 单一产品的私有袋。adapter 在往返中保留未知 namespace 与键 |
| `modules.*` | KnowledgeEntry、AssemblePacket 与 TimelineEvent 上的可选 `ModuleMap`（能力标志 `narrative-modules`）；键为功能方言 id（`activation`、`placement`、`activation_trace`、`cognition`、`mental`、`observation` 等）；值为结构化 JSON，内部形状由手册定义 | 叙事主机共享的跨产品功能方言。未知模块键原样往返 |

放置规则：**跨产品功能方言**用 `modules.*`；**产品数据**用 `extensions.<product>`。在 `HostCapabilityManifest` 上，`extensions` 携带部署元数据 —— roles、capabilities 与 namespace 所有权是核心 manifest 字段。

## 能力标志

| 标志 | 增加什么 |
|------|----------|
| `spoke-baseline` | 经五个 ops 线上族、`HostCapabilityManifest` + 基线 `HostManifestPort`、以及共享 `Scope` / `error-envelope` 定义获得 L0–L8 语义。基线合规可独立成立；可选标志是增量的 |
| `l2-computable` | KnowledgeEntry 上的 `body.state` / `body.computable`、`TimelineEvent.computable_logs`，以及 `project` / `compute` ops |
| `l5-fork` | TimelineEvent 上的 `fork_id` / `parent_fork_id` 分支元数据与 `Scope.fork_id` 过滤 |
| `spoke-cognition` | 标准可归因认知的单一家族（无子标志）：持有者 KnowledgeEntry 上的 `modules.cognition`（信念、epistemic 的 knowledge/ignorance/undetermined、感知记录，外加叙事世界事实；每条完整的信念、认知评估或感知记录携带必填基础 `q`，并可携带独立指标）、带归因保真投影的 `Scope.cognition` 选择、TimelineEvent 上的 `modules.observation`（可能感知的可达性）、记忆 `Content.cognition` 与 `cognition_filter`，以及派生的 `MindState` 认知。声明只覆盖主机实际提供的表面；不受支持的要求会显式拒绝 —— 见[认知记录](/zh/reference/data-model#cognition-spoke-cognition)与 [MindState 参考](/zh/reference/mind-state) |
| `narrative-modules` | KnowledgeEntry + AssemblePacket + TimelineEvent 上的可选 `modules`（`ModuleMap`）字段袋 |
| `ke-extraction` | 可选 `extract` 操作族 —— 从被引用的源材料提议 `provisional` 候选 KnowledgeEntry —— 外加独立 `ExtractionPort` 与 `orchestrateExtract` 边界 |
| `ke-ownership` | `KnowledgeEntry.owner` / `KnowledgeEntry.disclosure` 治理字段与共享 `Scope.viewpoint` 读取方选择器 |
| `spoke-connect` | 可选交互信封族；讲该协议的主机在 `HostCapabilityManifest.capabilities` 中列出该标志 |
| `spoke-memory` | 可选按主体记忆族：十三个模型与 33 个 `memory.*` 操作，外加独立 `MemoryPort`。两个可选的子标志 —— `spoke-memory-reflection`（3 个反思操作）与 `spoke-memory-transfer`（5 个导出/导入操作）—— 均要求 `spoke-memory`。见 [记忆参考](/zh/reference/memory) |

## 认知协商与拒绝 {#cognition-negotiation-and-rejection}

`spoke-cognition` 是单一能力标志。主机只有在实际提供的表面上实现全部强制规则后，才在 `HostCapabilityManifest.capabilities` 中列出它：仅提供 KnowledgeEntry 的主机实现认知模块、`modules.observation` 与 `Scope.cognition` 过滤/投影，无需记忆；同时提供 KnowledgeEntry 与记忆的主机还要满足记忆认知行（记录时点内容校验、过滤的 list/recall、反思依据、传输摘要、擦除）。manifest 字符串本身不是合规证据。

协商复用既有的签名握手交集：仅当两端 manifest 都声明时，`negotiatedCapabilities(local, remote)` 才授予 `spoke-cognition`。基线操作仍要求 `spoke-baseline`；载荷相关的门禁只在被派发的请求携带其声明载体时加入认知要求（`check`、`assemble`、`port.scope.list_knowledge_entries`、`port.scope.list_timeline_events`、`port.fork.list_timeline_events` 上的 `scope.cognition`；`memory.recall`、`memory.records.list` 与三个 `memory.reflections.*` 读取上的 `cognition_filter`）。协商集合缺少该族时，需要它的请求走既有 `capability_missing` 分支且无处理器副作用。未携带可选认知字段的记录仍是普通的缺失数据，不产生错误。

schema 解码后，未知谓词、已知但未实现的视图/维度/指标过滤，或在 KnowledgeEntry 载体上请求记录时点视图，都会通过既有错误信封拒绝整个请求：`code: "invalid_input"`、`message: "Unsupported cognition requirement"`、`details.reason: "unsupported_cognition_requirement"`，`details.field` 指出该要求的载体相对路径。响应中不做静默剥离、不忽略合取、不返回部分结果，也不回退到未过滤输出。

## 仓库布局

`schemas/`（SSOT）· `tooling/codegen/` · `packages/spoke-schemas` + `packages/spoke-operations`（TypeScript）· `crates/spoke-schemas` + `crates/spoke-operations`（Rust）· `fixtures/toy-world/`（一致性样例与参考 adapter）。

## 相关页面

- [数据模型参考](/zh/reference/data-model) —— 十一个数据对象的字段表。
- [操作线上参考（Ops wire）](/zh/reference/ops) —— 请求/响应信封、`Scope`、`ErrorEnvelope`。
- [connect 参考](/zh/reference/connect) —— 可选信封族。
- [认知记录](/zh/reference/data-model#cognition-spoke-cognition) —— `q`、可选指标、过滤语法与投影。
- [MindState 参考](/zh/reference/mind-state) —— 派生 L5 时间视图（`spoke-cognition`）。
- [记忆参考](/zh/reference/memory) —— 可选 `spoke-memory` 族：模型、操作、能力标志与提供方指引。
- [核心概念](/zh/explanation/concepts) —— 九层模型与能力标志如何映射。
- [`schemas/README.md`](https://github.com/42ch-dev/spoke/blob/main/schemas/README.md) —— schema 文件清单。
