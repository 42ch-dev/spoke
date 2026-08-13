---
title: MindState
---

# MindState

第一类时间心智状态记录（L5，可选 `l5-mind`）—— when 轴上的独立线上对象，记录持有者心智字段随时间如何变迁。必填：`schema_version`、`mind_state_id`、`holder_entry_id`、`extensions`。

## 何时使用

当心智状态的时间轨迹本身就是要交换的事实（错误信念结构、戏剧反讽结构，或任何"谁在时刻 t 相信 / 想要 / 感受什么"的记录）时使用 `MindState`。`MindState` 严格**派生** —— 持有者 KnowledgeEntry 的 `modules.mental` / `modules.belief`（既定归宿）仍是唯一权威，本记录绝不构成第二个事实源。

## 最小示例

```json
{
  "schema_version": 1,
  "mind_state_id": "ms_01HXYZ",
  "holder_entry_id": "kb_mira",
  "canonical_name": "Mira — relieved after the Treaty of Ashford",
  "occurred_at": "1421-06-03T12:00:00Z",
  "snapshot": {
    "emotions": [{ "emotion": "relief", "intensity": 0.7 }],
    "goals": [{ "goal": "secure the harbor charter", "status": "active" }]
  },
  "deltas": [
    {
      "path": "modules.mental.emotions",
      "previous": [{ "emotion": "anxious", "intensity": 0.8 }],
      "next": [{ "emotion": "relief", "intensity": 0.7 }]
    }
  ],
  "extensions": {}
}
```

## `l5-mind` 能力

`l5-mind` 是 L5 时间层上的可选能力标志 —— 声明它意味着产品实现 `MindState` 时间记录与 `TimelineEvent.modules` 上的 `modules.observation`。它不是 `spoke-baseline`；心智引擎（信念修订、ToM 推理、观察渲染）归产品所有。心智字段的既定归宿是持有者 KnowledgeEntry 上的 `modules.mental` / `modules.belief`（`narrative-modules` 字段袋）；`MindState` 只记录 when 轴上的快照 / 增量变化。

## 字段表与手册

字段级细节位于 spec 语料（SSOT）—— 此处不重复：

- [数据模型字段表 —— §MindState](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) —— 必填 / 可选字段、共享 `MentalFieldMap` / `MindDelta` 定义、双重关注表。
- [心智状态领域画像](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-mental-state.md) —— `modules.mental` 九字段词汇、`modules.belief` 标签空间、`modules.observation`、MindState 记录草图。
- [MindState schema](https://github.com/42ch-dev/spoke/blob/main/schemas/data/mind-state.schema.json) —— 已提交的线上 schema。

## 相关页面

- [数据模型参考](/zh/reference/data-model) —— 持久对象，含 TimelineEvent（when 轴）与 MindState / 本体标签之分。
- [协议参考](/zh/reference/protocol) —— 能力标志，含 `l5-mind`。
- [核心概念](/zh/explanation/concepts) —— 层与双重关注对。
