---
title: 协议总览
---

# 协议总览（Protocol umbrella）

SPOKE 是面向叙事产品的共享**线上方言（wire dialect）**：一套用于知识数据与操作的 JSON Schema 契约，使产品在共同的协议面上交换 KnowledgeEntry 数据与操作。协议横跨三列，外加一个可选加入的 connect 信封族。

本页为[英文原页](/guide/protocol)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 三列模型

- **数据线上（Data wire）** —— 八个持久对象：KnowledgeEntry、Relation、SourceAnchor、Finding、AssemblePacket、HostCapabilityManifest、Rule、TimelineEvent（`schemas/data/`，共享定义在 `schemas/common/`）。
- **操作线上（Ops wire）** —— 五个基线操作族（upsert、extract→promote、relate、check、assemble），以传输无关的请求/响应信封承载；`l2-computable` 下另有可选 `project` / `compute`。
- **操作库（Operations library）** —— 在生成的线上类型之上的手写行为层：纯函数生命周期辅助、按能力切片的 adapter ports 与注入式编排（TypeScript `@42ch/spoke-operations`；Rust `spoke-operations`，锁步 SemVer）。

## connect 信封族（可选加入）

六个交互信封（`schemas/connect/`，能力标志 `spoke-connect`）增加跨进程交互：以 `$ref` 内嵌 `HostCapabilityManifest` 的签名握手（hello）、会话上下文、将既有操作信封作为不透明载荷包装的 invoke 请求/响应，以及鉴权挑战/响应。

## 代码生成姿态

- `schemas/` 是唯一手写线上真源；生成的 TypeScript（`@42ch/spoke-schemas`）与 Rust（`spoke-schemas`）输出提交入库并镜像 schema 树。
- `pnpm run verify-codegen` 在生成树偏离 `schemas/` 时令构建失败；schema 变更与重新生成输出落在同一提交。
- 线上清单为 30 个已提交的 `*.schema.json`：2 common + 8 data + 14 ops + 6 connect 信封。
- 每个持久对象携带必填 `extensions.<namespace>`；核心字段保持封闭（`additionalProperties: false`）。

## 仓库布局

`schemas/`（SSOT）· `tooling/codegen/` · `packages/spoke-schemas` + `packages/spoke-operations`（TypeScript）· `crates/spoke-schemas` + `crates/spoke-operations`（Rust）· `fixtures/toy-world/`（一致性样例与参考 adapter）· `.mstar/specs/`（规范说明）。

## 规范参考

- [spoke-protocol.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol.md) —— 总览：问题框架、schema 清单、扩展、验收
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— 协议词汇
- [schemas/README.md](https://github.com/42ch-dev/spoke/blob/main/schemas/README.md) —— schema 文件清单
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) —— L0–L8 与能力等级
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) —— 数据对象
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) —— 操作线上
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) —— 操作库
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) —— connect 信封族
- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) —— 仓库概览与快速开始
