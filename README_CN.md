# SPOKE

[![CI](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml/badge.svg)](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Version](https://img.shields.io/github/v/release/42ch-dev/spoke?include_prereleases&sort=semver&label=version)](https://github.com/42ch-dev/spoke/releases)
[![Last commit](https://img.shields.io/github/last-commit/42ch-dev/spoke)](https://github.com/42ch-dev/spoke/commits/main)
[![Greptile: The War on Bugs](https://www.greptile.com/badge.svg)](https://www.greptile.com/?utm_source=oss_badge&utm_medium=readme&utm_campaign=greptile_for_open_source)

[English](README.md) · [Concepts](CONCEPTS.md) · [Strategy](STRATEGY.md) · [Contributing](CONTRIBUTING.md)

**Standardized Programmable Ontology Knowledge Engine** — 叙事 **KnowledgeEntry** 数据层与 **ops** 操作层的 JSON Schema 线上契约。各独立产品通过这些形状交换一致性检查与上下文组装的 I/O。

**包含：**

- 数据层 schema：KnowledgeEntry、Relation、SourceAnchor、Finding、AssemblePacket、**HostCapabilityManifest**、Rule、TimelineEvent
- Ops 层 schema：`upsert`、extract→promote、`relate`、`check`、`assemble`；可选 **`project` / `compute`**（`l2-computable` 能力下）
- 生成的 TypeScript（`@42ch/spoke-schemas`）与 Rust（`spoke-schemas`、`spoke-operations`）
- 纯函数生命周期辅助，以及 **adapter ports** 与 **injection orchestration**（`@42ch/spoke-operations` / `spoke-operations`）
- 可选 **Connect**，用于签名的跨进程交互（`@42ch/spoke-connect` / `spoke-connect`，以及原生绑定）
- 协议一致性样例与参考 **`ToyWorldAdapter`**（[`fixtures/toy-world/`](fixtures/toy-world/)）

## 软件包

已发布的消费方软件包共用**锁步 SemVer**。

| 软件包 | 注册表 | 职责 |
|--------|--------|------|
| [`@42ch/spoke-schemas`](https://www.npmjs.com/package/@42ch/spoke-schemas) | npm | 生成的 TypeScript 线上类型 — 描述**线上传输什么** |
| [`@42ch/spoke-operations`](https://www.npmjs.com/package/@42ch/spoke-operations) | npm | 基于上述类型的纯函数辅助、adapter ports 与编排 |
| [`@42ch/spoke-connect`](https://www.npmjs.com/package/@42ch/spoke-connect) | npm | 可选连接客户端 — 身份派生、hello 认证、会话核心、WebSocket 传输 |
| [`spoke-schemas`](https://crates.io/crates/spoke-schemas) | crates.io | 生成的 Rust 线上类型 |
| [`spoke-operations`](https://crates.io/crates/spoke-operations) | crates.io | 纯函数辅助、adapter ports 与编排 — 与 `@42ch/spoke-operations` 行为对齐 |
| [`spoke-connect`](https://crates.io/crates/spoke-connect) | crates.io | 可选连接参考 — libp2p 传输、会话核心、uniffi 绑定接口 |

产品专属载荷放在 `extensions.<namespace>` 下（namespace 键由产品自行选择）。多个叙事主机共享的跨产品功能方言（lore 激活、知识包、assemble 摆放）放在 `modules.*` 下 —— 这是 KnowledgeEntry 与 AssemblePacket 上一个可选、按能力启用的字段袋。见 [数据模型](https://spoke.42ch.dev/zh/reference/data-model)。

安装固定与软件包职责：[软件包快速开始](https://spoke.42ch.dev/zh/packages/quick-start)。

## 安装

### TypeScript（npm）

```bash
pnpm add @42ch/spoke-schemas @42ch/spoke-operations
# 将两者固定到同一锁步 SemVer，例如 @X.Y.Z
```

**`@42ch/spoke-schemas`** — 导入生成的线上类型：

```ts
import type {
  KnowledgeEntry,
  TimelineEvent,
  PromoteRequest,
  AssemblePacket,
  HostCapabilityManifest,
} from "@42ch/spoke-schemas";
```

**`@42ch/spoke-operations`** — 在同一 adapter 上实现按能力切片的 ports，再调用 `orchestrate*`：

```ts
import type { PromoteRequest, UpsertRequest } from "@42ch/spoke-schemas";
import {
  orchestrateUpsert,
  orchestratePromote,
  orchestrateCheck,
  orchestrateAssemble,
  type BaselineAdapter,
} from "@42ch/spoke-operations";

declare const adapter: BaselineAdapter; // 产品实现 BaselineAdapter / FullAdapter
declare const upsertRequest: UpsertRequest;
declare const promoteRequest: PromoteRequest;

const upserted = orchestrateUpsert(adapter, upsertRequest);
const promoted = orchestratePromote(adapter, promoteRequest);
```

可选能力使用 `ComputableAdapter` / `ForkAdapter`（或 `FullAdapter`），配合 `orchestrateProject`、`orchestrateCompute`、`orchestrateForkCheck`、`orchestrateForkAssemble`。纯函数辅助（`validatePromoteRequest`、`mergeExtensionMaps`、`buildAssemblePacket` 等）仍可用于聚焦门控。

### Rust（crates.io）

```bash
cargo add spoke-schemas spoke-operations
# 将两者固定到同一锁步 SemVer，例如 X.Y.Z
```

```toml
# Cargo.toml
[dependencies]
spoke-schemas = "X.Y.Z"
spoke-operations = "X.Y.Z"
```

**`spoke-schemas`** — 与 JSON Schema SSOT 对齐的线上类型：

```rust
use spoke_schemas::{KnowledgeEntry, HostCapabilityManifest, PromoteRequest, TimelineEvent};
```

**`spoke-operations`** — port traits 与 `orchestrate_*`（同时 re-export `spoke_schemas`）：

```rust
use spoke_operations::{
    orchestrate_promote, orchestrate_upsert, BaselineAdapter,
};
use spoke_operations::spoke_schemas::{PromoteRequest, UpsertRequest};

fn run_baseline(adapter: &impl BaselineAdapter, upsert: UpsertRequest, promote: PromoteRequest) {
    let _ = orchestrate_upsert(adapter, upsert);
    let _ = orchestrate_promote(adapter, promote);
}
```

### Connect（可选）

主机需要签名的跨进程交互（hello、session、invoke、auth 信封）时，声明 **`spoke-connect`** 能力。

**TypeScript** — 带 WebSocket 传输的 npm 客户端：

```bash
pnpm add @42ch/spoke-connect
# 与 schemas / operations 固定到同一锁步 SemVer
```

**Rust** — crates.io 参考实现，带 libp2p 传输与面向其他宿主语言的 uniffi 绑定面：

```bash
cargo add spoke-connect
```

Connect 支持多语言：npm 上的语言原生客户端（TypeScript）、crates.io 上的 Rust 参考实现（libp2p + uniffi 绑定面），以及经四种渠道发布的原生绑定 —— GitHub Packages NuGet/Maven、SPM git、Go modules git、PyPI。总览、TypeScript 路线与原生绑定见 [TypeScript 客户端](https://spoke.42ch.dev/zh/how-to/connect-ts-client) 与 [原生绑定](https://spoke.42ch.dev/zh/how-to/connect-native-bindings)。

## 版本与固定

在 npm 与 crates.io 上将各消费面固定到**同一** SemVer（`X.Y.Z`）：

```bash
pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z
cargo add spoke-schemas@X.Y.Z spoke-operations@X.Y.Z
```

带注释的 git 标签 `vX.Y.Z` 与该锁步版本一致。发布说明见 [`CHANGELOG.md`](CHANGELOG.md) 与 [GitHub Releases](https://github.com/42ch-dev/spoke/releases)。固定指南：[版本与发布](https://spoke.42ch.dev/zh/release/versioning)。

## 快速开始

在同一 adapter 类型上实现你声明的能力所对应的 port 族，再调用 `@42ch/spoke-operations` 的 `orchestrate*`（Rust 侧为 `orchestrate_*`）。

```typescript
import type { KnowledgeEntry, PromoteRequest } from "@42ch/spoke-schemas";
import {
  orchestratePromote,
  type BaselineAdapter,
} from "@42ch/spoke-operations";

// 产品 adapter 实现 BaselineAdapter。
// 参考 FullAdapter：fixtures/toy-world 的 ToyWorldAdapter
declare const adapter: BaselineAdapter;

const candidate: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "kb_01",
  entry_type: "character",
  canonical_name: "Aria",
  status: "provisional",
  body: { summary: "A reluctant scout." },
  extensions: {},
};

const request: PromoteRequest = { candidate };
const result = orchestratePromote(adapter, request);

if (result.ok) {
  // 已通过 adapter OCC ports 持久化 confirmed 条目
} else {
  console.error(result.code, result.message);
}
```

参考 **FullAdapter** 与已提交的「Mira at Harbor」样例图：[`fixtures/toy-world/`](fixtures/toy-world/)。分步软件包路径：[软件包快速开始](https://spoke.42ch.dev/zh/packages/quick-start)。

## 核心概念

| 术语 | 在 SPOKE 中的含义 |
|------|-------------------|
| **KnowledgeEntry** | 线上的原子叙事知识单元（`entry_id`、`entry_type`、`status`、`body`、`extensions`） |
| **Relation** | KnowledgeEntry 之间（或 KnowledgeEntry ↔ 来源）的有向边 |
| **SourceAnchor** | 指向手稿片段或外部定位器的溯源指针 |
| **Finding** | 一致性、风格或分析类检查器输出 |
| **Rule** | `check` 的声明式约束输入（L6） |
| **TimelineEvent** | when 轴上的第一类时间对象（L5） |
| **AssemblePacket** | 线上上下文组装载荷（供下游 LLM 提示的精简条目） |
| **HostCapabilityManifest** | 主机角色、能力与所拥有的 `namespaces[]`，用于进程内协作 |
| **Extensions** | 数据对象上的产品专属字段袋（`extensions.<namespace>`） |
| **Modules** | KnowledgeEntry + AssemblePacket 上的可选跨产品功能方言字段袋（按能力启用 `narrative-modules`） |
| **Adapter ports** | 注入式读写面（`KnowledgeEntryPort`、`HostManifestPort` 等），由产品负责持久化 |
| **Orchestration** | `orchestrate*` / `orchestrate_*` 序列：加载 scope、执行门控、经 ports 持久化 |

词汇与定位：[`CONCEPTS.md`](CONCEPTS.md)、[`STRATEGY.md`](STRATEGY.md)，以及 [核心概念](https://spoke.42ch.dev/zh/explanation/concepts)。

## 可选能力

需要可编程 KnowledgeEntry 体状态的产品可声明 **`l2-computable`**：

- **`body.state`** — 静态持久可计算值
- **`body.computable`** — 动态 Session 作用域投影
- **`TimelineEvent.computable_logs`** — Moment 层级字段变更展示
- **`project` / `compute` ops** — 初始化/投影与应用/结算 I/O 信封

需要 fork 作用域时间线查询的产品可声明 **`l5-fork`**。需要交换跨产品功能方言（lore 激活、知识包、assemble 摆放 / 激活轨迹）的产品可声明 **`narrative-modules`**：KnowledgeEntry 与 AssemblePacket 上的可选 `modules`（`ModuleMap`）字段袋承载这些方言，适配器对未知 module namespace 原样往返保留。内部形状由 Domain Profile 手册定义。

需要签名的跨进程交互的产品可声明 **`spoke-connect`**。组合后的 adapter 别名：`BaselineAdapter`、`ComputableAdapter`、`ForkAdapter`、`FullAdapter`。

基线集成方使用核心 schema；可选能力按需启用。细节：[核心概念](https://spoke.42ch.dev/zh/explanation/concepts)。

## 操作层

`@42ch/spoke-operations` / `spoke-operations` 提供纯函数辅助与经 ports 注入的编排：

- **基线编排：** `orchestrateUpsert`、`orchestratePromote`、`orchestrateRelate`、`orchestrateCheck`、`orchestrateAssemble`
- **可选编排：** `orchestrateProject`、`orchestrateCompute`、`orchestrateForkCheck`、`orchestrateForkAssemble`
- 按能力切片的 ports 与组合别名（`BaselineAdapter` … `FullAdapter`）
- 扩展与模块映射合并及往返保留（`mergeExtensionMaps` / `mergeModuleMaps`、`preserveExtensionMaps` / `preserveModuleMaps`）
- Finding / KnowledgeEntry `status` 迁移辅助
- 晋升接受与 upsert/relate 校验
- 由 KnowledgeEntry 构建 AssemblePacket
- 拒绝路径上统一的 `SpokeResult` / `SpokeRejectCode`

参考 **FullAdapter**（baseline + `l2-computable` + `l5-fork`，含 `HostCapabilityManifest` 对等主机）：[`fixtures/toy-world/`](fixtures/toy-world/) — TypeScript `ToyWorldAdapter`，Rust crate `spoke-fixture-toy-world`。

细节：[编排操作](https://spoke.42ch.dev/zh/how-to/orchestrate-ops)。

## 延伸阅读

| 主题 | 指南 |
|-------|------|
| 协议总览 | [协议](https://spoke.42ch.dev/zh/reference/protocol) |
| 九层模型与能力等级 | [核心概念](https://spoke.42ch.dev/zh/explanation/concepts) |
| 数据对象与开放词汇 | [数据模型](https://spoke.42ch.dev/zh/reference/data-model) |
| Ops 请求/响应信封 | [操作线上信封](https://spoke.42ch.dev/zh/reference/ops) |
| 操作库行为 | [编排操作](https://spoke.42ch.dev/zh/how-to/orchestrate-ops) |
| Core / modules / extensions | [数据模型](https://spoke.42ch.dev/zh/reference/data-model) |
| Connect 信封与绑定 | [连接](https://spoke.42ch.dev/zh/reference/connect) |
| Domain Profiles | [领域画像](https://spoke.42ch.dev/zh/explanation/domain-profiles) |
| JSON Schema SSOT | [`schemas/`](schemas/) |
| 参考 adapters 与样例图 | [`fixtures/toy-world/`](fixtures/toy-world/) |

## 贡献

本地开发、CI 门禁与发布流程见 [`CONTRIBUTING.md`](CONTRIBUTING.md)。
