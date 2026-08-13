---
title: 数据模型参考
---

# 数据模型参考（Data model reference）

数据层定义叙事产品交换的持久线上对象。所有对象传输无关、携带必填的 `extensions.<namespace>` 字段袋、并保持核心字段封闭（`additionalProperties: false`）。以下字段表溯源到 [`schemas/data/`](https://github.com/42ch-dev/spoke/tree/main/schemas/data) 与 [`schemas/common/`](https://github.com/42ch-dev/spoke/tree/main/schemas/common) 中的已提交 schema。

## 共享定义

| 定义 | 形状 | 说明 |
|------|------|------|
| `SchemaVersion` | 整数 ≥ 1 | 线上 schema 版本 |
| `Timestamp` | 字符串，RFC 3339 UTC | 创建 / 更新 / 发生时间 |
| `ExtensionMap` | 对象；键 `^[a-z][a-z0-9_-]*$`，值为不透明 JSON 对象 | 产品 namespace 袋；往返保留未知 namespace 与键 |
| `ModuleMap` | 对象；键 `^[a-z][a-z0-9_-]*$`，值为结构化 JSON（对象或数组） | 跨产品功能方言袋；往返保留未知模块 namespace |
| `SourceSpan` | `{ start, end }`（start 含、end 不含） | 源工件内的区间 |
| `TimelineScale` | 开放字符串；核心列表 `brief`、`narrative`、`moment` | L5 投影层级 |
| `ForkId` | 字符串 ≥ 1 字符 | 不透明世界历史分支标识（`l5-fork`） |
| `Scope` | 对象；必填 `scope_id` | 共享 ops 选择器 —— 见[操作线上参考](/zh/reference/ops) |
| `BodyAttribute` | `{ trait_type, value, display_type?, max_value? }` | ERC721 风格特征项；数组层允许重复 `trait_type` |
| `ComputableFieldMap` | 字段名到域值的开放映射 | `body.state` 与 `body.computable` 共用（`l2-computable`） |
| `ComputableLogEntry` | `{ logged_at, entry_id, changes[] }` + 可选 `session_id` / `message` | computable 字段变化的 moment 尺度呈现（`l2-computable`） |

## KnowledgeEntry

知识库原子单元。必填：`schema_version`、`entry_id`、`entry_type`、`canonical_name`、`status`、`body`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `entry_id` | 字符串 | 稳定 id，对协议不透明 |
| `entry_type` | 开放字符串 | 核心列表（记录在案，不强制）：`character`、`location`、`event`（本体标签 —— 区别于 TimelineEvent 线上对象）、`scene`、`act`、`organization`、`item`、`conflict`、`info_point`、`era`、`worldbuilding`、`note`、`research`、`ability`、`rule`（本体标签 —— 区别于 L6 Rule 线上对象）。产品可发布列表之外的值 |
| `canonical_name` | 字符串 ≥ 1 字符 | 人类稳定名称 |
| `status` | 开放字符串 | 核心列表（记录在案，不强制）：`provisional`、`confirmed`、`deprecated`、`merged`、`deleted` |
| `body` | 封闭对象 | `summary?`、`tags[]?`、`attributes[]?`（BodyAttribute）；`l2-computable` 下另有 `state?` / `computable?`（ComputableFieldMap） |
| `source_anchor` | SourceAnchor，可选 | 溯源指针 |
| `revision` | 整数 ≥ 0 | 乐观并发修订号 |
| `created_at` / `updated_at` | Timestamp | |
| `extensions` | ExtensionMap，必填 | |
| `modules` | ModuleMap，可选 | 能力标志 `narrative-modules`；携带按条目的方言（如 `modules.activation`） |

## Relation

两条 KnowledgeEntry（或一条 KnowledgeEntry 与一个源锚点）之间的有向边。必填：`schema_version`、`relation_id`、`relation_type`、`from_id`、`to_id`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `relation_id` | 字符串 | 稳定关系 id |
| `relation_type` | 开放字符串 | 核心列表（记录在案，不强制）：`related_to`、`parent_of`、`member_of`、`located_in`、`participates_in`、`causes`、`foreshadows` |
| `from_id` / `to_id` | 字符串 | 源 / 目标端点 id |
| `label` | 字符串，可选 | 人类可读标签 |
| `metadata` | 开放对象，可选 | |
| `revision` | 整数 ≥ 0 | 乐观并发修订号 |
| `extensions` | ExtensionMap，必填 | |

## SourceAnchor

指向源工件区间的指针（手稿、场景、外部定位符）。必填：`schema_version`、`source_id`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `source_id` | 字符串 | 不透明源定位符；语法由产品定义 |
| `span` | SourceSpan，可选 | 源内的字节或字符区间 |
| `label` | 字符串，可选 | 人类可读标签 |
| `mime_type` | 字符串，可选 | 所引用源的 MIME 类型 |
| `extensions` | ExtensionMap，必填 | |

## Finding

检查器输出 —— 与 KnowledgeEntry `body` 不同的独立工件。必填：`schema_version`、`finding_id`、`severity`、`status`、`title`、`description`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `finding_id` | 字符串 | 稳定 finding id |
| `severity` | 开放字符串 | 核心列表（记录在案，不强制）：`info`、`warning`、`error` |
| `status` | 开放字符串 | 核心列表（记录在案，不强制）：`open`、`resolved`、`dismissed` |
| `title` / `description` | 字符串 | 短标题与详情文本 |
| `kind` | 字符串，可选 | 检查器种类或类别 |
| `target_entry_id` | 字符串，可选 | 该 finding 针对的 KnowledgeEntry |
| `source_anchor` | SourceAnchor，可选 | 溯源指针 |
| `suggested_fix` | 字符串，可选 | 建议的修复文本 |
| `text_position` | 对象，可选 | 源文本内的位置提示 |
| `extensions` | ExtensionMap，必填 | |

## AssemblePacket

仅线上（wire-only）的上下文组装载荷。必填：`schema_version`、`packet_id`、`entries`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `packet_id` | 字符串 | 稳定包 id |
| `entries` | 数组 | 精简上下文条目（默认）；完整 KnowledgeEntry 内嵌按 op 决定 |
| `extensions` | ExtensionMap，必填 | |
| `modules` | ModuleMap，可选 | 能力标志 `narrative-modules`；携带包级配方（`modules.placement`、`modules.activation_trace`） |

## HostCapabilityManifest

进程内协作的主机自描述。必填：`schema_version`、`host_id`、`roles`、`capabilities`、`namespaces`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `host_id` | 字符串 ≥ 1 字符 | 稳定主机标识，对协议不透明 |
| `roles` | 字符串数组，≥1，去重 | 开放词汇。核心列表（记录在案，不强制）：`data-store`、`input-source`、`checker`、`assembler`、`computable-engine` |
| `capabilities` | 字符串数组，≥1，去重 | 开放字符串能力标志。核心列表（记录在案，不强制）：`spoke-baseline`、`l2-computable` |
| `namespaces` | 字符串数组，≥1，去重；键 `^[a-z][a-z0-9_-]*$` | 该主机在协作上下文中拥有的扩展 namespace 键 |
| `authority` | `{ scope_key }`，可选 | 显式单写者权限作用域；缺省且 `roles` 含 `data-store` 时，隐式权限为该 manifest 的 `host_id` |
| `extensions` | ExtensionMap，必填 | 部署元数据 —— 与 KnowledgeEntry `extensions` 是不同表面 |

## Rule

`check` 的声明式约束输入 —— 绝不是检查器输出。必填：`schema_version`、`rule_id`、`canonical_name`、`kind`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `rule_id` | 字符串 | 稳定规则 id，对协议不透明 |
| `canonical_name` | 字符串 ≥ 1 字符 | 人类稳定名称 |
| `kind` | 开放字符串 | 核心列表（记录在案，不强制）：`rule`、`prohibition`、`style` |
| `statement` | 字符串，可选 | 声明式约束文本（人类或机器可读；语法由产品选择） |
| `target_entry_types` | 字符串数组，可选 | 匹配 KnowledgeEntry `entry_type` 词汇的本体过滤 |
| `severity_hint` | 开放字符串，可选 | 核心列表（记录在案，不强制）：`info`、`warning`、`error` |
| `status` | 开放字符串，可选 | 核心列表（记录在案，不强制）：`draft`、`active`、`deprecated` |
| `source_anchor` | SourceAnchor，可选 | 规则锚定到手稿时使用 |
| `extensions` | ExtensionMap，必填 | |

## TimelineEvent

第一类 when 轴时间对象（L5）。必填：`schema_version`、`timeline_event_id`、`canonical_name`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `timeline_event_id` | 字符串 | 稳定 id，对协议不透明 |
| `canonical_name` | 字符串 | 人类稳定标签 |
| `timeline_scale` | TimelineScale，可选 | 投影层级：`brief`、`narrative`、`moment` |
| `occurred_at` | 字符串 | RFC 3339 或不透明模糊标签（如 "Third Age"） |
| `description` | 字符串，可选 | 更长的叙事摘要 |
| `participant_entry_ids` | 字符串数组，可选 | 相关的 KnowledgeEntry id |
| `source_anchor` | SourceAnchor，可选 | |
| `sort_key` | 字符串，可选 | 时间轴内的不透明排序提示 |
| `fork_id` / `parent_fork_id` | ForkId，可选 | 世界历史分支元数据（`l5-fork`） |
| `computable_logs` | ComputableLogEntry[]，可选 | computable 变化的 moment 尺度历史（`l2-computable`） |
| `modules` | ModuleMap，可选 | 能力标志 `narrative-modules`；携带事件观察元数据（`l5-mind` 下的 `modules.observation`） |
| `extensions` | ExtensionMap，必填 | |

`MindState` 是同一 when 轴上心智状态的配套 L5 时间记录 —— 见 [MindState 参考](/zh/reference/mind-state)。

## 开放词汇

`entry_type`、`relation_type`、statuses、severities 与 `kind` 都是**带记录在案核心列表的开放字符串** —— schema 保持开放，核心列表作为参考值。产品发布自己的值；Domain Profile（领域画像）记录已发布词汇（例如画像独有的 `entry_type: "beat"`）；adapter 原样往返未知值。

## 独立工件

- Rule 是声明式检查器**输入**；Finding 是检查器**输出** —— 各司其职。
- `TimelineEvent` 是 L5 when 轴对象；`entry_type: "event"` 是本体的标签 —— 一个本地概念可以同时映射到两者（双重关注）。
- `MindState` 是 L5 时间心智状态记录（`l5-mind`）；`entry_type: "character"` / 画像 `mind` 是本体的标签 —— 该记录严格派生自持有者的 `modules.mental` / `modules.belief`（既定归宿），绝不构成第二个权威。
- `HostCapabilityManifest` 在其专属表面上携带主机元数据（roles、capabilities、namespaces），与 KnowledgeEntry `extensions` 分离。

## 相关页面

- [协议参考](/zh/reference/protocol) —— schema 清单、扩展契约、能力标志。
- [操作线上参考（Ops wire）](/zh/reference/ops) —— `Scope` 与读取这些对象的 ops。
- [核心概念](/zh/explanation/concepts) —— 每个对象所属的层。
- [领域画像（Domain Profiles）](/zh/explanation/domain-profiles) —— 在这些形状之上发布的开放字符串词汇。
- [MindState 参考](/zh/reference/mind-state) —— L5 时间心智状态记录（`l5-mind`）。
