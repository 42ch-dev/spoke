---
title: 叙事结构
---

# Domain Profile —— 叙事结构（Narrative structure）

本 Domain Profile（领域画像）手册文档化基于既有 SPOKE 线上形状的 **Beat 辅助叙事大纲**：有序故事枢轴、场景原子与结构角色。它发布开放字符串词汇与映射指引 —— 核心 schema 保持不变。

本页为[英文原页](/profiles/narrative-structure)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## Beat 语义 → 线上映射

- **原子 / 场景 beat** —— `TimelineEvent`，`timeline_scale: "moment"`，可选与一条 KnowledgeEntry 配对（双重关注）。
- **结构 beat** —— KnowledgeEntry body 上的 `BodyAttribute`，`trait_type: "structural_role"`，例如 `midpoint`、`catalyst`、`finale`（profile 在 body-attribute trait 词汇中的开放字符串槽位标签）。
- **括号 beat** —— 剧本 `(beat)` 停顿映射到对话文本上的 `SourceAnchor` 片段。
- **排序** —— `Relation`，`relation_type: "precedes"`（或 `follows`），位于两条双重 KnowledgeEntry id 之间；moment 时间轴事件经 `extensions.spoke.timeline_entry_id` 链接其 KnowledgeEntry。
- **选择** —— `Scope` 过滤：`timeline_scale: "moment"`、`timeline_event_ids`，或包含 profile `beat` 标签的 `entry_types`。

Profile 专属 `entry_type: "beat"` 是本 profile 发布的合法开放字符串 —— 核心 `entry_type` 表与 schema 描述列表文档化核心词汇。

## 库支持

`@42ch/spoke-operations` / `spoke-operations` 导出 moment 层级过滤与 beat-sheet 排序辅助（`filterTimelineEventsByMomentScale`、`orderTimelineEventsByIds`、`orderTimelineEventsByPrecedes`）—— 在调用方提供数组上的纯函数，I/O 与存储由 adapter 处理。

## 规范参考

- [domain-profile-narrative-structure.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-narrative-structure.md) —— 完整映射矩阵、Relation 词汇、beat-sheet 互换样例
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— Domain Profile、双重关注、TimelineScale
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) —— TimelineEvent、BodyAttribute、Relation
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) —— Scope 细化
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) —— moment 过滤 / 排序辅助
