# SPOKE

[![CI](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml/badge.svg)](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Version](https://img.shields.io/github/v/release/42ch-dev/spoke?include_prereleases&sort=semver&label=version)](https://github.com/42ch-dev/spoke/releases)
[![Last commit](https://img.shields.io/github/last-commit/42ch-dev/spoke)](https://github.com/42ch-dev/spoke/commits/main)
[![Greptile: The War on Bugs](https://www.greptile.com/badge.svg)](https://www.greptile.com/?utm_source=oss_badge&utm_medium=readme&utm_campaign=greptile_for_open_source)

[English](README.md) · [Concepts](CONCEPTS.md) · [Strategy](STRATEGY.md) · [Contributing](CONTRIBUTING.md)

**Standardized Programmable Ontology Knowledge Engine** — 叙事 **KnowledgeEntry** 数据层与 **ops** 操作层的 JSON Schema 线上契约。各独立产品通过这些形状交换一致性检查与上下文组装的 I/O。

**包含：**

- 数据层 schema：KnowledgeEntry、Relation、SourceAnchor、Finding、AssemblePacket、Rule、TimelineEvent
- Ops 层 schema：`upsert`、extract→promote、`relate`、`check`、`assemble`；可选 **`project` / `compute`**（`l2-computable` 能力下）
- 生成的 TypeScript（`@42ch/spoke-schemas`）与 Rust（`spoke-schemas`、`spoke-operations`）
- 纯函数生命周期辅助库（`@42ch/spoke-operations` / `spoke-operations`）
- 协议一致性样例（[`fixtures/toy-world/`](fixtures/toy-world/)）

## 软件包

已发布的消费方软件包共用**锁步 SemVer**。

| 软件包 | 注册表 | 职责 |
|--------|--------|------|
| [`@42ch/spoke-schemas`](https://www.npmjs.com/package/@42ch/spoke-schemas) | npm | 生成的 TypeScript 线上类型 — 描述**线上传输什么** |
| [`@42ch/spoke-operations`](https://www.npmjs.com/package/@42ch/spoke-operations) | npm | 基于上述类型的纯函数生命周期辅助 |
| [`spoke-schemas`](https://crates.io/crates/spoke-schemas) | crates.io | 生成的 Rust 线上类型 |
| [`spoke-operations`](https://crates.io/crates/spoke-operations) | crates.io | 纯函数生命周期辅助 — 与 `@42ch/spoke-operations` 行为对齐 |

产品专属载荷放在 `extensions.<namespace>` 下（namespace 键由产品自行选择）。

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
} from "@42ch/spoke-schemas";
```

**`@42ch/spoke-operations`** — 调用纯函数辅助（依赖 `@42ch/spoke-schemas`）：

```ts
import type { PromoteRequest } from "@42ch/spoke-schemas";
import {
  validatePromoteRequest,
  applyPromoteAcceptance,
  buildAssemblePacket,
  transitionFindingStatus,
  mergeExtensionMaps,
} from "@42ch/spoke-operations";

const request: PromoteRequest = { candidate /* KnowledgeEntry */ };
const gate = validatePromoteRequest(request);
if (gate.ok) {
  const accepted = applyPromoteAcceptance(request);
  // 通过你的产品适配器持久化
}
```

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
use spoke_schemas::{KnowledgeEntry, PromoteRequest, TimelineEvent};
```

**`spoke-operations`** — 基于上述类型的辅助（同时 re-export `spoke_schemas`）：

```rust
use spoke_operations::{
    apply_promote_acceptance, validate_promote_request, SpokeResult,
};
use spoke_operations::spoke_schemas::PromoteRequest;

let gate = validate_promote_request(&request);
if let SpokeResult::Ok(_) = gate {
    let _accepted = apply_promote_acceptance(&request);
}
```

## 版本与固定

在 npm 与 crates.io 上将各消费面固定到**同一** SemVer（`X.Y.Z`）：

```bash
pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z
cargo add spoke-schemas@X.Y.Z spoke-operations@X.Y.Z
```

带注释的 git 标签 `vX.Y.Z` 与该锁步版本一致。发布说明见 [`CHANGELOG.md`](CHANGELOG.md) 与 [GitHub Releases](https://github.com/42ch-dev/spoke/releases)。

## 快速开始

```typescript
import type { KnowledgeEntry, PromoteRequest } from "@42ch/spoke-schemas";
import { validatePromoteRequest } from "@42ch/spoke-operations";

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
const result = validatePromoteRequest(request);

if (result.ok) {
  // 门控通过 — 通过你的产品适配器持久化
} else {
  console.error(result.code, result.message);
}
```

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
| **Extensions** | 数据对象上的产品专属字段袋（`extensions.<namespace>`） |

词汇与定位：[`CONCEPTS.md`](CONCEPTS.md)、[`STRATEGY.md`](STRATEGY.md)。

## 可选能力

需要可编程 KnowledgeEntry 体状态的产品可声明 **`l2-computable`**：

- **`body.state`** — 静态持久可计算值
- **`body.computable`** — 动态 Session 作用域投影
- **`TimelineEvent.computable_logs`** — Moment 层级字段变更展示
- **`project` / `compute` ops** — 初始化/投影与应用/结算 I/O 信封

基线集成方使用核心 schema；`l2-computable` 为可选能力。规范细节：[`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) §Capability levels。

## 操作层

`@42ch/spoke-operations` / `spoke-operations` 提供跨产品的纯函数生命周期辅助：

- 扩展映射合并与往返保留
- Finding `status` 迁移校验与应用
- 晋升接受检查（持久化前门控）
- 由 KnowledgeEntry 构建 AssemblePacket
- 拒绝路径上统一的 `SpokeResult` / `SpokeRejectCode`

规范细节：[`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md)。

## 规范与 schema

| 路径 | 主题 |
|------|------|
| [`schemas/`](schemas/) | JSON Schema 单一事实来源（Draft-07） |
| [`fixtures/toy-world/`](fixtures/toy-world/) | 协议一致性 JSON 图（「Mira at Harbor」） |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | 协议总览规范 |
| [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) | 九层模型（L0–L8）、能力等级、Timeline 层级 |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | 数据对象与开放词汇 |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Ops 线上请求/响应信封 |
| [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md) | 操作库行为 |

## 贡献

本地开发、CI 门禁与发布流程见 [`CONTRIBUTING.md`](CONTRIBUTING.md)。
