# SPOKE

[![CI](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml/badge.svg)](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Node](https://img.shields.io/badge/node-%3E%3D20-brightgreen.svg?logo=nodedotjs&logoColor=white)](package.json)
[![pnpm](https://img.shields.io/badge/pnpm-%3E%3D8-F69220.svg?logo=pnpm&logoColor=white)](package.json)
[![TypeScript](https://img.shields.io/badge/TypeScript-generated-3178C6.svg?logo=typescript&logoColor=white)](packages/spoke-schemas)
[![Rust](https://img.shields.io/badge/Rust-generated-DEA584.svg?logo=rust&logoColor=black)](crates/spoke-schemas)
[![Schema](https://img.shields.io/badge/JSON%20Schema-SSOT-0B7285.svg)](schemas)
[![Version](https://img.shields.io/badge/version-0.1.0-informational.svg)](package.json)
[![Last commit](https://img.shields.io/github/last-commit/42ch-dev/spoke)](https://github.com/42ch-dev/spoke/commits/main)
[![Greptile: The War on Bugs](https://www.greptile.com/badge.svg)](https://www.greptile.com/?utm_source=oss_badge&utm_medium=readme&utm_campaign=greptile_for_open_source)

[English](README.md) · [Concepts](CONCEPTS.md) · [Strategy](STRATEGY.md)

**Standardized Programmable Ontology Knowledge Engine** — 叙事 **KnowledgeEntry** 数据层与 **ops** 操作层的 JSON Schema 线上契约仓库。各独立产品用这些形状交换一致性检查与上下文组装的 I/O，无需共享数据库或运行时。

**包含：**

- 数据层 schema：KnowledgeEntry、Relation、SourceAnchor、Finding、AssemblePacket、Rule、TimelineEvent
- Ops 层 schema：`upsert`、extract→promote、`relate`、`check`、`assemble`；可选 **`project` / `compute`**（`l2-computable` 能力下）
- 生成的 TypeScript（`@42ch/spoke-schemas`）与 Rust（`spoke-schemas`）
- 纯函数生命周期辅助库（`@42ch/spoke-operations`）
- 协议一致性样例（[`fixtures/toy-world/`](fixtures/toy-world/)）

## 软件包

| 软件包 | 职责 |
|--------|------|
| [`@42ch/spoke-schemas`](packages/spoke-schemas/) | 由 JSON Schema 生成的 TypeScript 类型 — 描述**线上传输什么** |
| [`@42ch/spoke-operations`](packages/spoke-operations/) | 手写纯函数辅助 — 晋升门控、Finding 状态迁移、扩展合并、AssemblePacket 构建 |
| `spoke-schemas`（Rust crate） | [`crates/spoke-schemas/`](crates/spoke-schemas/) 中的生成 Rust 类型 |

产品专属载荷放在 `extensions.<namespace>` 下（namespace 键由产品自行选择）。

## 安装与消费

软件包为**工作区本地包**（private）。在 pnpm monorepo 中：

```json
{
  "dependencies": {
    "@42ch/spoke-schemas": "workspace:*",
    "@42ch/spoke-operations": "workspace:*"
  }
}
```

从其他仓库引用本地 checkout：

```json
{
  "dependencies": {
    "@42ch/spoke-schemas": "file:../spoke/packages/spoke-schemas",
    "@42ch/spoke-operations": "file:../spoke/packages/spoke-operations"
  }
}
```

然后在 SPOKE 根目录执行 `pnpm install`，并构建（`pnpm --filter @42ch/spoke-schemas build` 与 `pnpm --filter @42ch/spoke-operations build`）。

## 版本与固定

SPOKE 以**单一锁步 SemVer**（`X.Y.Z`）发布所有工作区软件包与 Rust `spoke-schemas` crate。在消费方仓库固定到带注释的 git 标签：

```bash
git checkout vX.Y.Z
```

或从对应的 [GitHub Release](https://github.com/42ch-dev/spoke/releases) 下载源码归档，再通过 `file:` 路径（上文）或 git 依赖引用：

```json
{
  "dependencies": {
    "@42ch/spoke-schemas": "github:42ch-dev/spoke#vX.Y.Z",
    "@42ch/spoke-operations": "github:42ch-dev/spoke#vX.Y.Z"
  }
}
```

文首 shields.io **Version** 徽章对应当前检出提交上的规范版本。规范政策：[`.mstar/specs/spoke-version-release.md`](.mstar/specs/spoke-version-release.md)。

## 发布（维护者）

当 `main` 上的变更就绪后：

1. Bump 所有锁步表面：`pnpm run release:bump -- X.Y.Z`
2.  提交并推送：`git add -A && git commit -m "chore(release): bump version to X.Y.Z" && git push`
3.  在干净提交上创建带注释标签 — 再次执行 `pnpm run release:bump -- X.Y.Z --tag "发布摘要"`，或 `git tag -a vX.Y.Z -m "发布摘要"`
4.  推送标签：`git push origin vX.Y.Z` — 触发 [`.github/workflows/release.yml`](.github/workflows/release.yml)，重新运行校验门禁并依据标签注释创建 GitHub Release。

预发布使用 `vX.Y.Z-rc.N` 标签（GitHub pre-release）。CI 仅创建 GitHub Release，不向 npm 或 crates.io 发布。

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

基线集成方可完整省略该能力，无破坏性变更。规范细节：[`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) §Capability levels。

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

其他导出包括 `buildAssemblePacket`、`transitionFindingStatus`、`mergeExtensionMaps` — 详见 [`spoke-operations.md`](.mstar/specs/spoke-operations.md)。

## 操作层

`@42ch/spoke-operations` 提供跨产品的纯函数生命周期辅助：

- 扩展映射合并与往返保留
- Finding `status` 迁移校验与应用
- 晋升接受检查（持久化前门控）
- 由 KnowledgeEntry 构建 AssemblePacket
- 拒绝路径上统一的 `SpokeResult` / `SpokeRejectCode`

规范细节：[`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md)。

## 规范与 schema

| 路径 | 主题 |
|------|------|
| [`schemas/`](schemas/) | JSON Schema 单一事实来源（Draft-07）— codegen 输入 |
| [`fixtures/toy-world/`](fixtures/toy-world/) | 协议一致性 JSON 图（「Mira at Harbor」）— CI schema 校验 |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | 协议总览规范 |
| [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) | 九层模型（L0–L8）、能力等级、Timeline 层级 |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | 数据对象与开放词汇 |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Ops 线上请求/响应信封 |
| [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md) | 操作库行为 |

## 贡献与 CI

Pull request 须通过 GitHub Actions 任务 `verify-codegen`、`typescript` 与 `rust`（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)）。修改 schema 时须在同一提交中重新生成产物（`pnpm run codegen`）。
