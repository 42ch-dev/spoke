---
title: 数据模型
---

# 数据模型（Data model）

数据层定义叙事产品交换的持久线上对象。所有对象传输无关、携带必填 `extensions.<namespace>`、核心字段保持封闭。

本页为[英文原页](/guide/data-model)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 对象

- **KnowledgeEntry（知识条目）** —— 身份、开放 `entry_type` / `status`、封闭 `body`（summary、tags、`attributes[]` 标量特征）、可选 `source_anchor`、必填 `extensions`。
- **Relation（关系）** —— 有向边（`from_id` / `to_id`），`relation_type` 开放；可选 `revision` 用于乐观并发。
- **SourceAnchor（溯源指针）** —— 来源产物片段指针。
- **Finding（检查器输出）** —— 检查器产物，`status` 词汇为 `open` / `resolved` / `dismissed`。
- **AssemblePacket（上下文组装载荷）** —— 仅线上的上下文组装载荷（精简 `entries[]`）。
- **HostCapabilityManifest（主机能力清单）** —— 主机角色、能力标志与所拥有的 namespace，用于进程内协作。
- **Rule（检查规则）** —— L6 `check` 的声明式约束输入（kind、statement、目标 entry 类型）。
- **TimelineEvent（时间轴事件）** —— L5 when 轴对象，带可选 `timeline_scale` 与（`l5-fork` 下）fork 字段。

## 开放词汇

`entry_type`、`relation_type` 与状态是**开放字符串**，带文档化核心列表 —— schema 保持开放，核心列表作为参考值。产品发出自己的值；Domain Profile（领域画像）文档化已发布词汇（例如 profile 专属 `entry_type: "beat"`）；adapter 对未知值原样往返。

## 扩展契约

每个持久对象携带 `extensions: { "<namespace>": { } }`。namespace 键是产品选择的 id；值是任意 JSON 对象。adapter 在每次读/写时保留未知 namespace 与 namespace 内未知键。

## 不同产物

- Rule 是声明式检查器输入；Finding 是检查器输出 —— 每个产物保持自己的角色。
- TimelineEvent 是 L5 when 轴对象；`entry_type: "event"` 是本体标签 —— 一个本地概念可映射到两者。
- 主机元数据位于 `HostCapabilityManifest`（roles、capabilities、namespaces）—— 专属清单面，与 KnowledgeEntry 的 `extensions` 分离。

## 规范参考

- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) —— 全部八个对象的字段表、扩展、开放词汇
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— 词汇与双重关注规则
- [schemas/README.md](https://github.com/42ch-dev/spoke/blob/main/schemas/README.md) —— schema 文件清单
- [schemas/data/](https://github.com/42ch-dev/spoke/tree/main/schemas/data) —— 已提交的数据 schema
