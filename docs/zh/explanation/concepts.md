---
title: 核心概念
---

# 核心概念（Concepts）

SPOKE 以**线上术语**定义词汇 —— 下面的每个概念都是协议面上的具体 JSON 形状或开放字符串词汇。本页是关键陈述导览：九层模型是什么、能力标志声明什么、协议刻意分开哪些概念对。字段级细节见[参考](/zh/reference/protocol)页面。

## 九层模型（L0–L8）

| 层 | 承载内容 |
|----|----------|
| **L0 信封** | 所有持久对象上的身份 + `schema_version` |
| **L1 本体** | 开放 `entry_type` 字符串 + Domain Profile（领域画像） |
| **L2 正文** | 封闭 `body`：summary、tags、标量 `attributes`（`l2-computable` 下另有可选 `state` / `computable`） |
| **L3 溯源** | SourceAnchor 指针 |
| **L4 图** | 类型化有向 Relation（可选 `revision` OCC） |
| **L5 时间** | TimelineEvent when 轴 + 投影层级 `brief` / `narrative` / `moment`（`l5-fork` 下可选 Fork；`l5-mind` 下可选 `MindState` 心智状态记录） |
| **L6 约束** | Rule 声明式 `check` 输入 |
| **L7 Finding** | 检查器输出 + 状态生命周期 |
| **L8 上下文** | AssemblePacket 线上形状（组装本身留在产品侧） |

## 能力等级

产品按声明的能力等级主张合规。**`spoke-baseline`** 经五个 ops 线上族、`HostCapabilityManifest` + 基线 `HostManifestPort`、以及共享 `Scope` / `error-envelope` 定义覆盖 L0–L8 语义。可选标志是增量的 —— 基线合规可独立成立：

- **`l2-computable`** —— computable 知识状态及其历史，以及 `project` / `compute` ops。
- **`l5-fork`** —— 时间轴上的世界历史分支元数据。
- **`l5-mind`** —— `MindState`，when 轴上严格派生的时间心智状态记录 —— 见 [MindState 参考](/zh/reference/mind-state)。
- **`narrative-modules`** —— 面向跨产品功能方言的可选 `modules` 字段袋。
- **`ke-extraction`** —— 可选 `extract` 操作：被引用的源材料进、`provisional` 候选 KnowledgeEntry 出。
- **`ke-ownership`** —— KnowledgeEntry 信封上的归属与披露，以及 `Scope.viewpoint` 读取方选择器。
- **`spoke-connect`** —— 可选跨进程交互信封族。
- **`spoke-memory`** —— 按主体记忆：十三个模型与 33 个 `memory.*` 操作，位于独立 `MemoryPort` 之后，另有可选的 `spoke-memory-reflection` 与 `spoke-memory-transfer` 子标志 —— 见 [记忆参考](/zh/reference/memory)。

connect 家族的会话生命周期、信封认证与能力路由在 [Connect 架构](/zh/explanation/connect) 中说明。

## 双重关注（dual-concern）对

协议刻意分开三组概念：

- KnowledgeEntry 上的本体 `entry_type: "event"` 标签与 L5 `TimelineEvent` when 轴对象 —— 一个本地概念可映射到其中一类或两类线上形状。
- 本体 `entry_type: "rule"` 标签与 L6 `Rule` 检查器输入 —— Rule 是声明式输入，Finding 是检查器输出。
- 本体 `entry_type: "character"` / 画像 `mind` 标签与 L5 `MindState` 时间记录 —— 标签对知识节点分类；`MindState` 是持有者心智字段（`modules.mental` / `modules.belief` 为既定归宿）的严格派生快照 / 增量。

名称保持分离，使 `check` / `assemble` 选择器保持无歧义。

## 记忆与既定知识并列

既定知识与主观记忆是两件不同的事，因此协议把它们放在两个族里。KnowledgeEntry 是其他主机可以 check、assemble 的共享、已审核节点；保留记忆有两条准入路径：捕获的材料先成为 `MemoryCandidate`，经 `memory.candidates.review` 后才计入；`memory.records.create` 则直接写入保留记录，属于作者管理面向。读取遵循调用方的授权范围。

| 方面 | KnowledgeEntry | 记忆记录 |
|------|----------------|----------|
| 角色 | 共享面上的既定知识节点 | 主机捕获的按主体印象 |
| 主体 | 非按主体划分 | 恰好一个 `subject_id`（扁平主体） |
| 准入 | `upsert`、`promote` | 捕获：`memory.capture` → `MemoryCandidate` → `memory.candidates.review`。作者管理写入：`memory.records.create` |
| 内容信任 | 信封携带 `ke-ownership` 披露字段 | 记录与候选内容是不受信数据；画像文档是受信作者材料 |
| 忘却 | 状态生命周期（`deprecated`、`merged`、`deleted`） | 仅显式操作：drop/dismiss、墓碑删除、主体归档、导入丢弃 |
| 读取轴 | 基于共享 `Scope` 的 `check` / `assemble` | `ReadSelection` 放置模式加 `ProvenanceFilter` |
| 授权 | 基于 `Scope.viewpoint` 的 `ke-ownership` 披露谓词 | 自包含的 `MemoryGrant` |

这一划分带来三个结果。情感以 `content.structured` 内部的不透明约定承载（例如 `{"affect":{"valence":0.6}}`），因此情绪分类学保持为产品策略。作者画像（`ProfileDocument`）承载精炼的作者基准，记录承载具体经历，反思则是跨记录的派生综合 —— 三层各司其职。提供方替换是一套线上流程：带版本与摘要的导出/导入，配以操作者定时的切换，全程保持单一权威。

## 开放词汇与 Domain Profile

核心对象保持封闭信封（`additionalProperties: false`），而核心词汇保持**开放**：`entry_type`、`relation_type` 与 statuses 都是带记录在案核心列表的开放字符串，而非封闭枚举。Domain Profile 在这些开放字符串之上发布本体词汇 —— beat 映射、lore 激活、知识包 —— 产品以开放字符串与 `extensions.<namespace>` 表达画像专属类型。见[领域画像（Domain Profiles）](/zh/explanation/domain-profiles)。

## 选择器与扩展点

- **Scope** —— `check` / `assemble` 的共享 ops 选择器：必填不透明 `scope_id` 加可选细化（`entry_ids`、`entry_types`、`timeline_scale`、`fork_id`、`viewpoint` 等）；`viewpoint` 是 `ke-ownership` 读取方选择器。
- **Extensions** —— 每个持久对象上的 `extensions.<namespace>` 产品字段袋；adapter 原样往返未知 namespace。
- **Modules** —— KnowledgeEntry、AssemblePacket 与 TimelineEvent 上的可选 `modules.*` 字段袋（能力标志 `narrative-modules`），用于跨产品功能方言。

## 纯函数库姿态

操作库是生成线上类型之上的纯行为层：生命周期辅助、按能力切片的 adapter ports 与注入式编排 —— 自身无任何 I/O。存储访问、LLM 调用、ranking、retrieval 与传输绑定由产品经注入 ports 提供；`fixtures/toy-world/` 中的参考 adapter 演示了这一模式。

## 相关页面

- [协议参考](/zh/reference/protocol) —— schema 清单、扩展契约、能力标志。
- [数据模型参考](/zh/reference/data-model) —— 每层对应的字段表。
- [领域画像（Domain Profiles）](/zh/explanation/domain-profiles) —— 已发布的开放字符串词汇。
- [MindState 参考](/zh/reference/mind-state) —— L5 时间心智状态记录（`l5-mind`）。
- [记忆参考](/zh/reference/memory) —— 可选 `spoke-memory` 族，以及它如何与既定知识并列。
