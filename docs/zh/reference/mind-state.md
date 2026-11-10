---
title: MindState
---

# MindState

第一类时间心智状态记录（L5，可选 `spoke-cognition`）—— when 轴上的独立线上对象，记录持有者心智状态随时间如何变迁。必填：`schema_version`、`mind_state_id`、`holder_entry_id`、`extensions`。

## 何时使用

当心智状态的时间轨迹本身就是要交换的事实（错误信念结构、戏剧反讽结构，或任何「谁在时刻 t 知道、相信、感知、想要或感受什么」的记录）时使用 `MindState`。`MindState` 严格**派生** —— 持有者 KnowledgeEntry 的 `modules.cognition`（标准认知记录）与 `modules.mental`（九字段心智词汇：`identity`、`beliefs`、`attention`、`goals`、`intentions`、`emotions`、`dispositions`、`norms`、`constraints`）承载权威值，本记录把它们在时间上呈现出来。`modules.mental.beliefs` 承载派生摘要、计数或引用数据。

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
      "path": "/modules/mental/emotions",
      "previous": [{ "emotion": "anxious", "intensity": 0.8 }],
      "next": [{ "emotion": "relief", "intensity": 0.7 }]
    }
  ],
  "extensions": {}
}
```

## 字段表

| 字段 | 类型 | 说明 |
|------|------|------|
| `schema_version` | SchemaVersion | 必填 |
| `mind_state_id` | 字符串 ≥ 1 字符 | 稳定 id，对协议不透明 |
| `holder_entry_id` | 字符串 ≥ 1 字符 | 心智角色的 KnowledgeEntry `entry_id`；派生来源 |
| `canonical_name` | 字符串 ≥ 1 字符，可选 | 人类稳定标签 |
| `occurred_at` | Timestamp，可选 | 该心智状态成立的时间 |
| `sort_key` | 字符串，可选 | 心智状态时间轴内的不透明排序提示 |
| `snapshot` | MentalFieldMap，可选 | 持有者心智状态在该时点的派生全字段快照 |
| `deltas` | MindDelta[]，可选 | 相对前一心智状态的派生字段级变更记录 |
| `cognition` | CognitionProjection，可选 | 所选持有者侧认知权威的派生投影，保留全部匹配评估与当前视图轨迹 |
| `source_anchor` | SourceAnchor，可选 | 手稿或场景锚点 |
| `created_at` / `updated_at` | Timestamp，可选 | |
| `extensions` | ExtensionMap | 必填 |

每个 `MindDelta.path` 都是 RFC 6901 JSON Pointer，根为 `/modules/mental/<field>`（九个字段之一）、`/modules/cognition/records/...` 或 `/modules/cognition/world_facts/...`。增量仅用于呈现。

## `spoke-cognition` 能力

`spoke-cognition` 是单一可选能力标志，无子标志。在该表面上，主机实现 `MindState` 时间记录、持有者 KnowledgeEntry 上的 `modules.cognition`、`TimelineEvent.modules` 上的 `modules.observation`，以及带归因保真投影的 `Scope.cognition` 选择。同时提供记忆的主机还实现记录时点 `Content.cognition` 校验与 `cognition_filter` 读取。声明只覆盖主机实际提供的表面，manifest 字符串本身不是合规证据；协商复用签名握手的交集，不受支持的要求会以 `details.reason: "unsupported_cognition_requirement"` 拒绝整个请求。心智引擎（信念修订、ToM 推理、观察渲染）归产品所有。见 [协议参考 —— 认知协商与拒绝](/zh/reference/protocol#cognition-negotiation-and-rejection)。

## 派生认知投影

`MindState.cognition` 携带所选持有者侧记录的 `CognitionProjection`。`current` 投影对每个已评估条目携带已标识的选择策略与输入/所选记录 id 轨迹；`recorded_at` 投影携带历史记录且没有当前选择。每个条目保留自己的 `assessment_context`，全部保留的匹配评估仍在 `assessments` 中，分歧被保留而非取平均。`assessed` 条目仅对过滤匹配到的记录出现。`no_record` 条目不是匹配记录：仅当过滤精确地址持有者、完整目标身份、`assessment_context.context_id` 与显式 fork 谓词，且授权来源在所选 `view` 下、该复合键上、可选谓词之前没有任何记录时才出现——省略 fork 即通配，普通空结果为 `items: []`（[数据模型 §选择与投影](/zh/reference/data-model#选择与投影)）。事件 `modules.observation` 说明谁可能感知；`PerceptionRecord` 说明持有者实际捕获了什么。

## 字段表与手册

字段级细节位于参考与 spec 表面：

- [数据模型参考 —— 认知](/zh/reference/data-model#cognition-spoke-cognition) —— 记录形状、`q` 与可选指标、过滤语法与投影规则。
- [数据模型字段表 —— §MindState](https://github.com/42ch-dev/spoke/blob/main/.agents/specs/protocol/spoke-data-model.md) —— 必填 / 可选字段与共享 `MentalFieldMap` / `MindDelta` 定义。
- [心智状态领域画像](https://github.com/42ch-dev/spoke/blob/main/.agents/specs/domain-profiles/domain-profile-mental-state.md) —— `modules.mental` 九字段词汇、`modules.observation`、MindState 记录草图。
- [MindState schema](https://github.com/42ch-dev/spoke/blob/main/schemas/data/mind-state.schema.json) —— 已提交的线上 schema。

## 相关页面

- [数据模型参考](/zh/reference/data-model) —— 持久对象，含 TimelineEvent（when 轴）、CognitionRecord 与 MindState / 本体标签之分。
- [协议参考](/zh/reference/protocol) —— 能力标志、协商与拒绝。
- [核心概念](/zh/explanation/concepts) —— 层与双重关注对。
