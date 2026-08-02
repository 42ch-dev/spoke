---
title: 操作线上信封（Ops wire）
---

# 操作线上信封（Ops wire）

操作层为核心 KnowledgeEntry 操作定义传输无关的请求/响应信封。产品通过任意传输承载这些 JSON 载荷 —— 进程内调用、消息队列或 adapter 内的 HTTP 映射 —— 线上保持传输无关。

本页为[英文原页](/guide/ops-wire)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 基线操作

- **upsert** —— 按稳定 id 创建或更新 KnowledgeEntry（1..n 条；可选幂等键）。
- **extract→promote** —— 将抽取候选晋升为持久 KnowledgeEntry（`promote-*` 族；可选合并目标）。
- **relate** —— 创建或更新 Relation。
- **check** —— 在 `Scope` 上运行检查器并返回 `Finding[]`（规则由 `rule_refs` 和/或内嵌 `rules[]` 提供）。
- **assemble** —— 为 `Scope` 返回 `AssemblePacket`（仅结构）。

每个操作都有配对的请求与响应 schema。可选的 `project` / `compute` 操作族（`l2-computable` 下）增加 Session（可计算生命周期）作用域的可计算 I/O。

## 共享规则

- **Scope（查询范围）选择器** —— `check` 与 `assemble` 要求共享 `Scope`：不透明 `scope_id` 加可选细化（`entry_ids`、`entry_types`、`timeline_event_ids`、`source_id`、`timeline_scale`、`fork_id`、`extensions`）。世界 / 书 / 产品 id 经 `extensions` 或 adapter 映射。
- **单一失败方言** —— 每个响应是成功载荷或 `{ "error": ErrorEnvelope }` 的 `oneOf`；成功与失败分支互斥。
- **Check ≠ Assemble** —— `check` 只返回 findings；`assemble` 只返回载荷。
- **`$ref` 组合** —— ops schema 以 `$ref` 引用数据层类型，每个类型只定义一次。

## 最小请求示例

```json
// 仅示意 —— 以 schemas/ops/ 下已提交的 schema 为准。
{
  "scope": { "scope_id": "book-harbor", "entry_types": ["character"] },
  "rules": [
    { "rule_id": "r-1", "kind": "rule", "canonical_name": "foreshadow", "extensions": {} }
  ]
}
```

## 规范参考

- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) —— 各操作完整契约、Scope、错误信封、可选操作
- [schemas/ops/](https://github.com/42ch-dev/spoke/tree/main/schemas/ops) —— 已提交的 ops schema
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) —— 线上之上的生命周期门控与编排
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— Scope 与 error-envelope 词汇
