---
title: 核心概念
---

# 核心概念（Concepts）

SPOKE 用**线上（wire）术语**定义其词汇 —— 下文每个概念都是协议面上的具体 JSON 形状或开放字符串词汇。权威定义见 [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md)；本页是导览，帮助您为集成选择正确的产物。

本页为[英文原页](/guide/concepts)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 核心对象

- **KnowledgeEntry（知识条目）** —— 知识库的原子条目：稳定 `entry_id`、开放 `entry_type` / `status` 字符串、封闭 `body`（summary、tags、标量 traits）、可选溯源、必填 `extensions`。
- **Relation（关系）** —— 两条 KnowledgeEntry（或一条 KnowledgeEntry 与一个来源）之间的有向边，可选 `revision` 用于乐观并发。
- **SourceAnchor（溯源指针）** —— 指向来源产物片段（手稿、场景、外部定位器）。
- **Finding（检查器输出）** —— 一致性、风格、结构类检查器输出，与 KnowledgeEntry 的 body 相区别。
- **Rule（检查规则）** —— `check`（L6）的声明式约束输入，与本体标签相区别。
- **TimelineEvent（时间轴事件）** —— when 轴上的第一类时间对象（L5），可选 `timeline_scale` 层级 `brief` / `narrative` / `moment`。
- **AssemblePacket（上下文组装载荷）** —— 仅线上的上下文组装载荷：供下游消费的精简条目。
- **HostCapabilityManifest（主机能力清单）** —— adapter 自描述：`host_id`、`roles`、`capabilities`、所拥有的 `namespaces`。

## 选择器与扩展点

- **Scope（查询范围）** —— `check` / `assemble` 共享的操作选择器：必填不透明 `scope_id` 加可选细化（`entry_ids`、`entry_types`、`timeline_scale`、`fork_id` 等）。
- **Domain Profile（领域画像）** —— 集成方通过开放字符串发布本体词汇的方式（beat 映射、lore activation、知识包）；核心枚举保持开放。
- **Extensions（扩展字段袋）** —— 每个持久对象上的 `extensions.<namespace>` 产品字段袋；adapter 对未知 namespace 原样往返。
- **Modules（模块字段袋）** —— 可选的 `modules.*` 字段袋（能力标志 `narrative-modules`），承载 KnowledgeEntry 与 AssemblePacket 上的跨产品功能方言。

## 双重关注对（Dual-concern）

SPOKE 有意将两对概念分开：KnowledgeEntry 上的本体标签 `entry_type: "event"` 与 L5 `TimelineEvent` when 轴对象；本体标签 `entry_type: "rule"` 与 L6 `Rule` 检查器输入。一个本地概念可映射到其中一个或两个线上形状；名称保持分离，使 check / assemble 选择器保持无歧义。

## 可选能力

- **`l2-computable`** —— `body.state` / `body.computable`、`TimelineEvent.computable_logs`，以及 `project` / `compute` 操作。
- **`l5-fork`** —— TimelineEvent 上的 `fork_id` / `parent_fork_id` 世界历史分支元数据。
- **`narrative-modules`** —— 共享功能方言的可选 `modules` 字段袋。
- **`spoke-connect`** —— 可选加入的跨进程交互信封族。

## 规范参考

- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— 词汇 SSOT、双重关注规则、拼写
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) —— 每个数据对象的字段语义
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) —— 分层模型与能力等级
- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) —— core / modules / extensions 归属权威
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) —— connect 词汇（`peer_id`、capability token）
