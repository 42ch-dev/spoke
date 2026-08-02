---
title: 分层与能力
---

# 分层与能力（Layers & capabilities）

协议将线上方言组织为**九个概念层**（L0–L8），从信封身份到上下文载荷。产品在声明的能力等级上宣称符合：**基线**（`spoke-baseline`）覆盖必需层，可选能力标志扩展特定层。

本页为[英文原页](/guide/layers)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 分层一览

- **L0 Envelope（信封）** —— 所有持久对象上的身份 + `schema_version`
- **L1 Ontology（本体）** —— 开放 `entry_type` 字符串 + Domain Profile（领域画像）
- **L2 Body** —— 封闭 `body`：summary、tags、标量 `attributes`（`l2-computable` 下可选 `state` / `computable`）
- **L3 Provenance（溯源）** —— SourceAnchor（溯源指针）
- **L4 Graph（图）** —— 类型化有向 Relation（可选 `revision` OCC）
- **L5 Temporal（时间）** —— TimelineEvent（时间轴事件）when 轴 + 投影层级 `brief` / `narrative` / `moment`（`l5-fork` 下可选 Fork）
- **L6 Constraint（约束）** —— Rule（检查规则）对 `check` 的声明式输入
- **L7 Finding** —— 检查器输出 + 状态生命周期
- **L8 Context（上下文）** —— AssemblePacket（上下文组装载荷）线上形状（组装本身留在产品侧）

## 能力等级

- **`spoke-baseline`** —— 通过五个操作线上族实现 L0–L8 语义、`HostCapabilityManifest` + 基线 `HostManifestPort`、共享 `Scope` / `error-envelope` 定义。可选能力标志是加性的 —— 基线符合独立成立。
- **`l2-computable`** —— 可选 `body.state` / `body.computable`、`TimelineEvent.computable_logs`，以及 `project` / `compute` 操作。
- **`l5-fork`** —— TimelineEvent 上可选的 `fork_id` / `parent_fork_id` 分支元数据，以及 `Scope.fork_id` 过滤。
- **`narrative-modules`** —— KnowledgeEntry + AssemblePacket 上可选的 `modules`（`ModuleMap`），承载跨产品功能方言。
- **`spoke-connect`** —— 可选加入的交互信封族；声明该能力的主机在 `HostCapabilityManifest.capabilities` 中列出此标志。

## 硬边界

- Rule 是声明式输入；Finding 是检查器输出 —— 两个不同的产物。
- `check` 返回 findings；`assemble` 返回载荷 —— 每个操作返回自己的产物。
- Timeline 层级（`brief` / `narrative` / `moment`）是协议 when 轴标签，区别于 L8 上下文组装与 Fork 分支。

## Domain Profile（领域画像）

Domain Profile 通过核心线上形状发布本体词汇：profile 类型表位于 adapter 规范与手册（narrative-structure beat 映射、lore activation）。核心对象保持封闭信封（`additionalProperties: false`），而核心词汇保持开放 —— `entry_type`、`relation_type` 与状态是带文档化核心列表的开放字符串，而非封闭枚举。产品用开放字符串与 `extensions.<namespace>` 表达 profile 专属类型。

## 规范参考

- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) —— 完整分层表、能力等级、层 ↔ 产物映射
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— spoke-baseline、Domain Profile、能力标志
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) —— 各层字段表
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) —— 各层对应的操作线上
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) —— connect 能力标志
