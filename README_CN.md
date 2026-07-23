# SPOKE

[English](README.md)

**Standardized Programmable Ontology Keyblock Engine** — 叙事 **Keyblock** 数据层与 **ops** 操作层的 JSON Schema 线上契约仓库。Nexus、Creader 等产品用这些形状交换一致性检查与上下文组装的 I/O。

**包含：** 数据层 schema（Keyblock、Relation、SourceAnchor、Finding、AssemblePacket）；ops 层 schema（`upsert`、extract→promote、`relate`、`check`、`assemble`）；生成的 TypeScript（`@42ch/spoke-schema`）与 Rust（`spoke-schema`）；纯函数生命周期辅助库（`@42ch/spoke-operations`）。

## 软件包

| 软件包 | 职责 |
|--------|------|
| [`@42ch/spoke-schema`](packages/spoke-schema/) | 由 JSON Schema 生成的 TypeScript 类型 — 描述**线上传输什么** |
| [`@42ch/spoke-operations`](packages/spoke-operations/) | 手写纯函数辅助 — 晋升门控、Finding 状态迁移、扩展合并、AssemblePacket 构建 |
| `spoke-schema`（Rust crate） | [`crates/spoke-schema/`](crates/spoke-schema/) 中的生成 Rust 类型 |

产品专属载荷放在 `extensions.<namespace>` 下（例如 `extensions.nexus`、`extensions.creader`）。

## 安装与消费

软件包为**工作区本地包**（private）。在 pnpm monorepo 中：

```json
{
  "dependencies": {
    "@42ch/spoke-schema": "workspace:*",
    "@42ch/spoke-operations": "workspace:*"
  }
}
```

从其他仓库引用本地 checkout：

```json
{
  "dependencies": {
    "@42ch/spoke-schema": "file:../spoke/packages/spoke-schema",
    "@42ch/spoke-operations": "file:../spoke/packages/spoke-operations"
  }
}
```

然后在 SPOKE 根目录执行 `pnpm install`，并构建（`pnpm --filter @42ch/spoke-schema build` 与 `pnpm --filter @42ch/spoke-operations build`）。

## 核心概念

| 术语 | 在 SPOKE 中的含义 |
|------|-------------------|
| **Keyblock** | 线上的原子叙事知识单元（`keyblock_id`、`block_type`、`status`、`body`、`extensions`） |
| **Relation** | Keyblock 之间（或 Keyblock ↔ 来源）的有向边 |
| **SourceAnchor** | 指向手稿片段或外部定位器的溯源指针 |
| **Finding** | 一致性、风格或分析类检查器输出 |
| **AssemblePacket** | 线上上下文组装载荷（供下游 LLM 提示的精简条目） |
| **Extensions** | 数据对象上的产品专属字段袋（`extensions.<namespace>`） |

词汇与定位：[`CONCEPTS.md`](CONCEPTS.md)、[`STRATEGY.md`](STRATEGY.md)。

## 快速开始

```typescript
import type { Keyblock, PromoteRequest } from "@42ch/spoke-schema";
import { validatePromoteRequest } from "@42ch/spoke-operations";

const candidate: Keyblock = {
  schema_version: 1,
  keyblock_id: "kb_01",
  block_type: "character",
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
- 由 Keyblock 构建 AssemblePacket
- 拒绝路径上统一的 `SpokeResult` / `SpokeRejectCode`

规范细节：[`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md)。

## 规范与 schema

| 路径 | 主题 |
|------|------|
| [`schemas/`](schemas/) | JSON Schema 单一事实来源（Draft-07）— codegen 输入 |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | 协议总览规范 |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | 数据对象与开放词汇 |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Ops 线上请求/响应信封 |
| [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md) | 操作库行为 |

## 贡献与 CI

Pull request 须通过 GitHub Actions 任务 `verify-codegen`、`typescript` 与 `rust`（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)）。修改 schema 时须在同一提交中重新生成产物（`pnpm run codegen`）。
