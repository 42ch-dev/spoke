---
title: 数据模型参考
---

# 数据模型参考（Data model reference）

数据层定义叙事产品交换的持久线上对象。对象传输无关；必填的 `extensions.<namespace>` 字段袋与核心字段封闭规则详见[扩展契约](/zh/reference/protocol#扩展契约)。以下字段表溯源到 [`schemas/data/`](https://github.com/42ch-dev/spoke/tree/main/schemas/data) 与 [`schemas/common/`](https://github.com/42ch-dev/spoke/tree/main/schemas/common) 中的已提交 schema。

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
| `Scope` | 对象；必填 `scope_id`；可选 `viewpoint`（`ke-ownership`）、`cognition`（`spoke-cognition`） | 共享 ops 选择器 —— 见[操作线上参考](/zh/reference/ops)、[归属治理](#归属治理-ke-ownership)与[认知](#cognition-spoke-cognition) |
| `BodyAttribute` | `{ trait_type, value, display_type?, max_value? }` | ERC721 风格特征项；数组层允许重复 `trait_type` |
| `ComputableFieldMap` | 字段名到域值的开放映射 | `body.state` 与 `body.computable` 共用（`l2-computable`） |
| `ComputableLogEntry` | `{ logged_at, entry_id, changes[] }` + 可选 `session_id` / `message` | computable 字段变化的 moment 尺度呈现（`l2-computable`） |
| `MentalFieldMap` | 心智字段名到域值的开放映射 | `MindState.snapshot`；匹配 `modules.mental` 九字段词汇 |
| `MindDelta` | `{ path, previous?, next? }` | `MindState.deltas[]`；`path` 为 RFC 6901 JSON Pointer，根为 `/modules/mental/<field>`、`/modules/cognition/records/...` 或 `/modules/cognition/world_facts/...` |

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
| `modules` | ModuleMap，可选 | 能力标志 `narrative-modules`；携带按条目的方言（如 `modules.activation`）；`modules.cognition` 在 `spoke-cognition` 下携带封闭认知模块 |
| `owner` | 字符串 ≥ 1 字符，可选 | 产品协作上下文中持有者 KnowledgeEntry 的 `entry_id`（`ke-ownership`） |
| `disclosure` | 开放字符串，可选 | 治理披露词汇；唯一核心值为 `owner-private`（`ke-ownership`） |

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
| `cognition` | CognitionProjection，可选 | 由 `Scope.cognition` 选出的派生投影（`spoke-cognition`） |

## HostCapabilityManifest

进程内协作的主机自描述。必填：`schema_version`、`host_id`、`roles`、`capabilities`、`namespaces`、`extensions`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `host_id` | 字符串 ≥ 1 字符 | 稳定主机标识，对协议不透明 |
| `roles` | 字符串数组，≥1，去重 | 开放词汇。核心列表（记录在案，不强制）：`data-store`、`input-source`、`checker`、`assembler`、`computable-engine` |
| `capabilities` | 字符串数组，≥1，去重 | 开放字符串能力标志。核心列表（记录在案，不强制）：`spoke-baseline`、`l2-computable`、`l5-fork`、`spoke-cognition`、`narrative-modules`、`ke-extraction`、`ke-ownership`、`spoke-connect` |
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
| `modules` | ModuleMap，可选 | 能力标志 `narrative-modules`；`modules.observation` 承载可能感知的观察者与开放的感知可达性约束（`spoke-cognition`） |
| `extensions` | ExtensionMap，必填 | |

`MindState` 是同一 when 轴上心智状态的配套 L5 时间记录 —— 见 [MindState 参考](/zh/reference/mind-state)。

## 认知（`spoke-cognition`） {#cognition-spoke-cognition}

标准认知是一个可选族 —— 单标志 `spoke-cognition`，无子标志。主机通过持有者 KnowledgeEntry 上的封闭 `modules.cognition` 模块提供它：`records[]` 承载持有者侧的完整记录，`world_facts[]` 承载叙事世界事实，模块自带 `extensions`。四个概念落在三类记录上。信念是 `BeliefRecord`。认知评估是单个 `EpistemicAssessment`，其 `state` 区分 Knowledge（`known`）、Ignorance（`ignorant`）与评估者不确定（`undetermined`）。感知是 `PerceptionRecord`，描述持有者实际捕获的内容。感知与信念是各自独立的记录，可对同一持有者与目标并存。

### 记录形状

每条完整的信念、认知评估与感知记录都携带 `q`，其 `assertion` 指出该 `q` 所估计的认知断言。`record_id` 在其载体内唯一。

| 记录（`kind`） | 必填核心 | 可选 |
|----------------|----------|------|
| 信念（`belief`） | `record_id`、`holder`、`target`、`assessor`、`assessor_role`、`basis`、`assessment_context`、`effective_at`、`assessed_at`、`view`、`order`、`q`、`extensions` | `criterion`、`metrics`、`truth`、`access`、`representation`、`content_type`、`source`、`context` |
| 认知评估（`epistemic`） | `record_id`、`holder`、`target`、`assessor`、`assessor_role`、`basis`、`assessment_context`、`effective_at`、`assessed_at`、`view`、`state`（`known` / `ignorant` / `undetermined`）、`criterion`、`classification_policy`（`policy_id`、`policy_version`）、`q`、`extensions` | `metrics` |
| 感知（`perception`） | `record_id`、`holder`、`target`、`assessor`、`assessor_role`、`basis`、`assessment_context`、`effective_at`、`assessed_at`、`view`、`episode`、`content`（必填键）、`q`、`extensions` | `access`、`criterion`、`metrics` |
| 世界事实（`world_facts[]`） | `fact_id`、`holder`（kind 为 `world`）、`target`、`order` = 0、`assessment_context`、`effective_at`、`recorded_at`、`extensions` | `truth`、`access`、`representation`、`content_type`、`source`、`context` |

世界事实位于指定的世界状态 KnowledgeEntry 上；其 `holder` 是世界权威，绝不为角色。角色侧的信念、认知评估与感知记录位于持有者 KnowledgeEntry 的 `modules.cognition.records[]`。

### 身份、归因与 `q`

| 元素 | 形状与规则 |
|------|------------|
| 引用 | `{ kind, id, namespace }`；`kind` 取 `knowledge_entry`、`timeline_event`、`memory_record`、`memory_subject`、`principal`、`world`、`external`。身份是完整的 `(kind, namespace, id)` 元组 —— 绝不仅是裸 id 或相同文本 |
| 目标 | `target.carrier`（引用）、`target_id`（载体内稳定）、`target.kind`（`proposition`、`question`、`perceived_content`、`episode`）、可选 `text`（表意内容，不作身份）、可选 `about`（高阶目标） |
| 复合目标身份 | `(carrier, kind, target_id, assessment_context.context_id, assessment_context.fork_id)`。不存在目标注册表或查找操作 |
| `assessment_context` | 必填 `context_id`，可选 `fork_id`；过滤谓词以 `{eq}` 选一个 fork，或以 `{absent:true}` 选无 fork 的上下文 |
| `assessor` / `assessor_role` | 一个引用加 `holder_self_report`、`external_judgment` 或 `product_assessment`。归因说明由谁评估；它不授予对其他评估的权威 |
| `basis` | `{ kind: "evidence", evidence: [引用...] }`（至少一个引用），或 `{ kind: "prior", description }`（非空声明的 subjective/prior/model 依据）。两者均可携带 `acquisition`。声明的依据可以保持未校准 |
| `q` | `{ value (0–1), assertion, method, calibration, uncertainty? }`。`assertion` 为 `belief_attribution`、`actual_capture`、`knowledge_criterion`、`ignorance_criterion` 或 `assessor_indeterminacy`；`calibration` 为 `calibrated`、`uncalibrated` 或 `unknown`；`uncertainty` 为 `{ kind, lower, upper, level?, description? }` |
| 估计量身份 | 持有者、目标、评估者、依据、适用时的 criterion、评估上下文、`effective_at` / `assessed_at` 与 `q.assertion` 共同标识被估计的概率 |
| 时间 | `effective_at` 是被评估状态适用的时间；`assessed_at` 是评估者作出评估的时间 |
| `view` | `current` 表示持有者侧的当前权威；`recorded_at` 表示记忆内容与显式提供的历史输入 |

信念 `q` 是该信念记录的归因置信度；持有者自身的信度是独立的可选指标 `b`。感知 `q` 估计实际捕获；同一事件的可达性元数据与可选对应性指标 `f` 各自独立。认知评估 `q` 随 `state`：`known` 用 `knowledge_criterion`，`ignorant` 用 `ignorance_criterion`，`undetermined` 用 `assessor_indeterminacy`。

### 可选指标

指标是彼此独立的估计。仅当该估计存在时键才出现，且每个指标重复其归因身份（`assessor`、`assessor_role`、`basis`、`context`、`effective_at`、`assessed_at`、`method`、`calibration`）与其 `estimand`。

| 键 | `estimand` | 允许类别 |
|----|------------|----------|
| `b` | `holder_credence` | 信念 |
| `p` | `capture` | 感知 |
| `f` | `correspondence` | 感知 |
| `k` | `knowledge_criterion` | 认知评估 |
| `r` | `recording_fidelity` | 信念、认知评估、感知 |

`knowledge_criterion` 与 `correspondence` 估计需要 `criterion`。产品本地度量放入 `extensions.<product>`。

### 信念词汇

`order` 为 `0`–`3`（高阶深度；当前角色信念记录使用 `1`–`3`，`order` 为 `0` 的叙事事实使用世界事实）。七个封闭维度是 `order`、`truth`（`True` / `False` / `Unknown`）、`access`（`Private` / `Shared` / `Public`）、`representation`（`Explicit` / `Implicit`）、`content_type`（`Location`、`Contents/Physical State`、`Identity/Relation`、`Epistemic`、`Desire/Intention`、`Emotion`、`Trait/Value`、`Action/Event`）、`source`（`Narration`、`Perception`、`Memory`、`Testimony`、`Inference`、`Imagination`、`Unknown`）与 `context`（`Deceptive`、`Temporal`、`Counterfactual`、`Neutral`）。`truth: "Unknown"` 表示记录的真值，不表示持有者无知；`access` 是分布标签，绝不构成授权；`source: "Perception"` 是获取路径标签，不证明实际感知发生过。

### 观察与实际感知

`TimelineEvent.modules.observation` 是封闭的 `ObservationModule`：必填 `observers`，承载可能感知该事件的角色 `CognitionReference`；`access` 承载开放的感知可达性约束（`position`、`line_of_sight`、`hearing_range`、`modality`，未知键在往返中保留）。缺失 `modules.observation` 表示没有观察元数据；`observers: []` 明确表示没有观察者可能感知。`PerceptionRecord` 描述实际捕获的内容 —— 包含部分、误导、错觉或不被相信的内容 —— 事件可达性本身不创建感知记录，也不提供默认概率。

### 选择与投影

`Scope.cognition` 承载 `CognitionSelection`。`current` 形式为 `{ view: "current", filter, selection_policy }`，要求协商得到的 `spoke-cognition` 标志；它只适用于持有者侧的当前权威。`recorded_at` 形式为 `{ view: "recorded_at", filter }`，适用于显式提供的历史输入；KnowledgeEntry 载体提供 `current` 视图，记忆提供 `recorded_at`。携带 `Scope.cognition` 或 `cognition_filter` 的请求要求协商得到的标志。

过滤使用一套封闭语法。每个提供的谓词都必须在同一条记录上成立：

| 谓词 | 匹配 |
|------|------|
| `holder` | 完整的持有者引用 |
| `target` | `carrier`（完整引用）、`target_id`、`kind` |
| `assessment_context` | `context_id`；`fork_id` 以 `{ eq }` 或 `{ absent: true }` 表示 |
| `category` | `belief`、`epistemic` 或 `perception` |
| `state` | 认知评估的 `state` |
| `q` | 对声明的 `q.value` 做含端点比较的 `min` / `max`；已声明但未校准的值同等参与 |
| `order` | 信念 `order` |
| `truth`、`access`、`representation`、`content_type`、`source`、`belief_context` | `{ eq }` 匹配存在的值，`{ absent: true }` 匹配显式缺失的可选字段 |

谓词绝不跨记录或跨目标连接，引用谓词比较完整元组。语法中没有 OR、文本相似度、排序、默认阈值、可选指标过滤或隐式来源/校准/criterion 约束。schema 未知的谓词、未实现的维度或指标过滤、或不可用的请求视图会使整个请求以 `unsupported_cognition_requirement` 拒绝，`details.field` 指出该要求的位置。

`CognitionProjection` 为 `{ view, items[], extensions }`。每个条目携带 `holder`、`target`、`assessment_context`、`presence`（`assessed` 或 `no_record`）与 `assessments[]` —— 所有保留下来的已授权匹配记录，分歧保留，绝不取平均或被替换。`current` 的已评估条目还携带 `selection`：`{ policy: { policy_id, policy_version }, input_record_ids[], selected_record_id }`，所选 id 是已记录输入 id 之一，策略由产品提供。`recorded_at` 投影没有当前选择。仅当过滤精确地址持有者、完整目标身份、`assessment_context.context_id` 与显式 fork 谓词，且已授权来源在该复合键上没有记录（在可选谓词之前）时，才输出 `presence: "no_record"`；省略 fork 是通配，不会产生 `no_record` 声明。普通空结果为 `items: []`。

### 记忆认知

记忆 `Content.cognition` 是可选的标准完整记录数组；每条记录携带 `view: "recorded_at"`，并保留自身的归因、依据与时间。成员缺失、显式出现的空数组与非空数组是三种不同的表示：缺失表示未记录标准认知；`[]` 作为独立的线上值保留，绝不由省略产生；只有非空数组携带认知实质 —— 显式空数组绝不会变成缺失，也绝不计为非空内容。非空数组作为认知内容计入分类；而每个显式存在的成员（包括 `[]`）都按原样保留，并按既有内容字节预算计量。认知在捕获以及记录创建或更新时校验。凡适用时复核默认使用捕获时的提议内容；`promote` 决定可提供新的 `content_override`，`fragment` 决定携带各片段的新 `content`，两者都按同一 recorded-at-time 规则在任何效果发生前独立校验。既有授权、放置与家族边界保持不变，复核不创建 KnowledgeEntry，待复核不授予任何权威。线上形状与复核输入见[记忆参考](/zh/reference/memory#认知内容与过滤-spoke-cognition)。

`memory.recall` 与 `memory.records.list` 上的 `cognition_filter` 是硬过滤，在已授权来源集解析之后、计数、排序、分页与字节预算之前应用：当记录至少有一条 `Content.cognition` 条目满足完整合取时匹配。该过滤绑定游标身份。`memory.reflections.state`、`memory.reflections.list` 与 `memory.reflections.generate` 接受同一过滤，并将其与来源 id + 修订集、profile/policy 身份及游标身份一并固定进反思依据，因此过滤或来源集变化会改变依据与陈旧判定。导出/导入的 bundle 与 chunk 将认知纳入规范摘要，`(kind, namespace, id)` 引用翻译要求完整引用并在不满足时失败关闭。删除、擦除与墓碑会连同其余敏感内容一起清除认知；`memory.records.promote` 只改变放置，不创建 KnowledgeEntry。

### 派生 MindState

`MindState.cognition` 携带所选持有者侧权威的派生 `CognitionProjection`，`MindState.snapshot` / `deltas[]` 呈现心智状态轨迹。每个 `MindDelta.path` 都是 RFC 6901 JSON Pointer，根为 `/modules/mental/<field>`、`/modules/cognition/records/...` 或 `/modules/cognition/world_facts/...`。权威值仍保留在持有者 KnowledgeEntry 上。

## 归属治理（`ke-ownership`）

可选的 `ke-ownership` 能力在 KnowledgeEntry 信封上承载归属事实，并在共享 `Scope` 上提供读取方选择器。`KnowledgeEntry.owner` 是产品协作上下文中持有者 KnowledgeEntry 的 `entry_id`；`owner` 缺省表示未指定归属。`KnowledgeEntry.disclosure` 承载治理披露词汇，唯一核心值为 `owner-private`；`disclosure` 缺省表示在已选定的 KB 上下文内共享。持有者是产品在自身上下文中解析的普通 KnowledgeEntry。

`Scope.viewpoint` 是共享选择器上的可选非空字符串：读者持有者 KnowledgeEntry 的 `entry_id`。核心披露谓词仅在条目的 `owner` 与请求的 `viewpoint` 精确相等时准入 `owner-private` 条目 —— 对 id 原样做字符串相等比较；没有 `disclosure` 的条目对任何 viewpoint 都保持可见。

三个带 Scope 的远程操作 —— `port.scope.list_knowledge_entries`、`port.scope.list_timeline_events`、`port.fork.list_timeline_events` —— 在请求 Scope 携带非空 `viewpoint` 时，除行能力外还要求 `ke-ownership`。见 [connect 参考](/zh/reference/connect#归属门禁-the-ownership-gate-ke-ownership)。

## 开放词汇

`entry_type`、`relation_type`、statuses、severities 与 `kind` 都是**带记录在案核心列表的开放字符串** —— schema 保持开放，核心列表作为参考值。产品发布自己的值；Domain Profile（领域画像）记录已发布词汇（例如画像独有的 `entry_type: "beat"`）；adapter 原样往返未知值。

## 独立工件

- Rule 是声明式检查器**输入**；Finding 是检查器**输出** —— 各司其职。
- `TimelineEvent` 是 L5 when 轴对象；`entry_type: "event"` 是本体的标签 —— 一个本地概念可以同时映射到两者（双重关注）。
- `MindState` 是 L5 时间心智状态记录（`spoke-cognition`）；`entry_type: "character"` / 画像 `mind` 是本体的标签 —— 该记录严格派生自持有者的 `modules.cognition` / `modules.mental`，权威值保留在持有者 KnowledgeEntry 上。
- `CognitionRecord` 是标准记录形状（`spoke-cognition`）；`modules.mental` 承载开放的九字段心智词汇，其 `beliefs` 成员是派生摘要、计数或引用数据。
- `modules.observation` 说明谁可能感知某事件；`PerceptionRecord` 说明持有者实际捕获了什么。
- `HostCapabilityManifest` 在其专属表面上携带主机元数据（roles、capabilities、namespaces），与 KnowledgeEntry `extensions` 分离。

## 相关页面

- [协议参考](/zh/reference/protocol) —— schema 清单、扩展契约、能力标志。
- [操作线上参考（Ops wire）](/zh/reference/ops) —— `Scope` 与读取这些对象的 ops。
- [核心概念](/zh/explanation/concepts) —— 每个对象所属的层。
- [领域画像（Domain Profiles）](/zh/explanation/domain-profiles) —— 在这些形状之上发布的开放字符串词汇。
- [MindState 参考](/zh/reference/mind-state) —— 派生 L5 时间记录（`spoke-cognition`）。
- [认知（`spoke-cognition`）](#cognition-spoke-cognition) —— 记录形状、`q` 与指标、过滤语法、投影与记忆载体。
