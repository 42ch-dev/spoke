---
title: 操作线上参考（Ops wire）
---

# 操作线上参考（Ops wire reference）

操作层为核心 KnowledgeEntry 操作定义传输无关的请求/响应信封。产品可经任意传输承载这些 JSON 载荷 —— 进程内调用、消息队列或 adapter 内的 HTTP 映射 —— 线上保持传输无关。以下字段表溯源到 [`schemas/ops/`](https://github.com/42ch-dev/spoke/tree/main/schemas/ops) 中的已提交 schema。

## 基线操作

| Op | 请求 | 响应 | 语义 |
|----|------|------|------|
| `upsert` | `UpsertRequest` | `UpsertResponse` | 按稳定 id 创建或更新 KnowledgeEntry（1..n 条；可选幂等键） |
| `extract→promote` | `PromoteRequest` | `PromoteResponse` | 把提取的候选提升为持久 KnowledgeEntry（可选合并目标） |
| `relate` | `RelateRequest` | `RelateResponse` | 创建或更新 Relation |
| `check` | `CheckRequest` | `CheckResponse` | 在一个 `Scope` 上运行检查器；返回 `Finding[]`（规则经 `rule_refs` 和/或内嵌 `rules[]`） |
| `assemble` | `AssembleRequest` | `AssembleResponse` | 为 `Scope` 返回 `AssemblePacket`（仅结构） |

每个操作都有配对的请求与响应 schema。`l2-computable` 下的可选 `project` / `compute` 族增加会话级 computable I/O。

## 单一失败方言

每个响应都是成功载荷或 `{ "error": ErrorEnvelope }` 的 `oneOf` —— 成功与错误分支互斥：

| ErrorEnvelope 字段 | 类型 | 说明 |
|---------------------|------|------|
| `code` | 字符串，必填 | 机器可读错误码（开放词汇） |
| `message` | 字符串，必填 | 人类可读错误消息 |
| `details` | 开放对象，可选 | 结构化错误上下文 |
| `extensions` | ExtensionMap，必填 | 产品 namespace 袋 |

操作库的预期拒绝以 `SpokeResult` 返回，携带 TypeScript 与 Rust 共享的稳定 `SpokeRejectCode` 字符串（`REVISION_CONFLICT`、`STORED_REVISION_STALE`、`CANDIDATE_NOT_PROVISIONAL`、`CANDIDATE_TERMINAL_STATUS`、`EMPTY_CANONICAL_NAME`、`RELATION_SELF_EDGE`、`RELATION_MISSING_ENDPOINT`、`CAPABILITY_PORT_MISSING`、`INTERNAL_ERROR` 等）。

## Scope 选择器

`check` 与 `assemble` 共享 `Scope` 选择器。`scope_id` 必填；其余细化全部可选：

| 字段 | 类型 | 说明 |
|------|------|------|
| `scope_id` | 字符串，必填 | 协议中立的不透明选择器。产品经 adapter 或 op extensions 映射 World / Book / 章节 / 手稿 id |
| `entry_ids` | 字符串数组 | 收窄到显式 KnowledgeEntry |
| `entry_types` | 字符串数组 | 按开放 `entry_type` 词汇过滤 |
| `timeline_event_ids` | 字符串数组 | 收窄到显式 L5 TimelineEvent id |
| `source_id` | 字符串 | 溯源或手稿定位符作用域 |
| `timeline_scale` | TimelineScale | L5 层级过滤（`brief`、`narrative`、`moment`） |
| `fork_id` | ForkId | L5 分支过滤 —— 对 `TimelineEvent.fork_id` 严格相等（`l5-fork`） |
| `viewpoint` | 字符串 | 读者上下文（`ke-ownership`）—— 读者的 holder KnowledgeEntry `entry_id`；缺省时不指名主体，也不授予私有可见性 |
| `extensions` | ExtensionMap | 产品作用域查询元数据；协议匹配器忽略它，adapter 原样往返 |

## 信封字段表

### UpsertRequest / UpsertResponse

`UpsertRequest` 必填：`knowledge_entries`。响应：`{ knowledge_entries: [...] }` **或** `{ error }`。

| 字段 | 说明 |
|------|------|
| `knowledge_entries` | 要创建或更新的 KnowledgeEntry |
| `idempotency_key` | 不透明幂等提示（协议 v0.1 无服务端语义） |
| `extensions` | 可选传输元数据 |

### PromoteRequest / PromoteResponse

`PromoteRequest` 必填：`candidate`。响应：`{ knowledge_entry, superseded_id? }` **或** `{ error }`。

| 字段 | 说明 |
|------|------|
| `candidate` | 候选 KnowledgeEntry（通常 `status` 为 `provisional`） |
| `target_entry_id` | 可选合并目标 KnowledgeEntry id；响应随之携带 `superseded_id` |

### RelateRequest / RelateResponse

`RelateRequest` 必填：`relation`。响应：`{ relation }` **或** `{ error }`。

| 字段 | 说明 |
|------|------|
| `relation` | 要创建或更新的 Relation（经 `revision` 做 OCC） |

### CheckRequest / CheckResponse

`CheckRequest` 必填：`scope`。响应：`{ findings: [...] }` **或** `{ error }`。

| 字段 | 说明 |
|------|------|
| `scope` | 检查器作用域选择器 |
| `rule_refs` | 不透明规则 id 或 URI；未被 `rules[]` 覆盖时由接收方解析 |
| `rules` | 可选内嵌 Rule 对象，用于可移植交换（按 `rule_id` 覆盖） |
| `checker_kinds` | 可选检查器种类过滤 |
| `extensions` | 可选传输元数据 |

### AssembleRequest / AssembleResponse

`AssembleRequest` 必填：`scope`。响应：`{ packet }` **或** `{ error }`。

| 字段 | 说明 |
|------|------|
| `scope` | 组装作用域选择器 |
| `max_entries` | 可选条目数量提示（协议不强制） |
| `extensions` | 可选传输元数据 |

## 可选操作（`ke-extraction`）

`extract` 是 `ke-extraction` 能力标志下的可选操作族，由既有 `input-source` 角色的主机提供；五个基线操作族保持不变。它从被引用的源材料提议 `provisional` 候选 KnowledgeEntry。

| Op | 意图 | 请求 | 响应 |
|----|------|------|------|
| `extract` | 从被引用的源材料提议 `provisional` 候选 KnowledgeEntry | `ExtractRequest` | `ExtractResponse` |

### ExtractRequest / ExtractResponse

`ExtractRequest` 必填：`run_id`、`sources`。输入仅含引用：请求携带溯源指针与各自的可选 span。

| 字段 | 说明 |
|------|------|
| `run_id` | 非空不透明关联标识；经 `run.run_id` 原样回显 |
| `sources` | 非空 `SourceAnchor[]`（溯源指针）；抽取范围是该列表加上每个锚点的可选 span（span 缺省表示整个被引用制品） |
| `entry_types` | 可选的咨询性提示，说明调用方期望的候选类型；服务方主机可采用或忽略它们 |
| `extensions` | 可选传输元数据 |

`ExtractResponse` 携带单一分支：成功 `{ candidates, run }` —— **或** 失败 `{ error }`。

| 成功字段 | 说明 |
|----------|------|
| `candidates` | `KnowledgeEntry[]` —— 每个返回的候选都携带 `status: "provisional"`；空数组是合法的零结果运行 |
| `run` | `ExtractionRunMetadata` —— 必填 `run_id`（请求的精确回显）、可选非空 `method`、可选不透明 `coverage_hint` |
| `extensions` | 可选传输元数据 |

| 失败字段 | 说明 |
|----------|------|
| `error` | `ErrorEnvelope` —— 该响应的失败分支 |
| `extensions` | 可选传输元数据 |

**`provisional` 不变量。** 库只准入 `status` 为 `provisional` 的候选。当任一候选携带其他状态时，整个集合被拒绝并返回错误分支 —— `merged` / `deleted` 条目给出 `CANDIDATE_TERMINAL_STATUS`，其他状态给出 `CANDIDATE_NOT_PROVISIONAL`；被准入的候选保持抽取器产出的状态，成功响应并原样回显请求的 `run_id`。

**`extract` 与 `extract→promote` 的区别。** 基线 `extract→promote` 行覆盖准入：`promote` 把单个 `provisional` 候选准入持久存储。可选 `extract` 操作覆盖产出：它从被引用的源材料提议 `provisional` 候选。抽取产物经 promote 到达持久存储。

## 共享规则

- **Check ≠ Assemble** —— `check` 只返回 findings；`assemble` 只返回包。
- **`$ref` 组合** —— ops schema 以 `$ref` 引用数据层类型，每个类型只定义一次。
- **纯度** —— 操作库相对主机 I/O 是纯函数：存储访问、LLM 调用、ranking、retrieval 与传输绑定由产品经注入的 adapter ports 提供。库运行协议门禁；adapter 拥有持久化。

## 相关页面

- [协议参考](/zh/reference/protocol) —— 三列模型与能力标志。
- [编排操作（orchestrate ops）](/zh/how-to/orchestrate-ops) —— 用真实签名调用每个编排器。
- [数据模型参考](/zh/reference/data-model) —— 这些信封携带的对象。
- [connect 参考](/zh/reference/connect) —— ops 信封作为不透明 invoke 载荷被包装。
