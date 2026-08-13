---
title: 领域画像（Domain Profiles）
---

# 领域画像（Domain Profiles）

**Domain Profile（领域画像）**在核心线上形状之上发布本体词汇：画像专属的 `entry_type` 标签、`relation_type` 值、body 特征（trait）与 `modules.*` 方言。核心 schema 保持不变 —— 词汇经由开放字符串与可选 `modules` 字段袋承载。本页记录四个画像；每节列出集成方可以发出与消费的已发布开放字符串词汇。

## 叙事结构（Narrative structure）

基于既有 SPOKE 线上形状的**节拍辅助叙事大纲（Beat-assisted narrative outlining）**：有序的故事转折、场景原子与结构角色。

| 已发布词汇 | 线上归宿 | 值 |
|------------|----------|-----|
| 画像条目类型 | `KnowledgeEntry.entry_type` | `beat`（画像专属标签，合法开放字符串） |
| 结构角色 | `KnowledgeEntry.body.attributes[].trait_type` | `structural_role`，槽位如 `midpoint`、`catalyst`、`finale` |
| 节拍排序 | `Relation.relation_type` | 双 KnowledgeEntry id 之间的 `precedes`（或 `follows`） |
| moment 层级 | `TimelineEvent.timeline_scale` / `Scope.timeline_scale` | `moment` —— 原子 / 场景节拍即该层级的 TimelineEvent |
| 节拍-条目链接 | `TimelineEvent.extensions.spoke` | 指向双关注 KnowledgeEntry 的 `timeline_entry_id` |
| 括注停顿 | `SourceAnchor.span` | 对白文本上的剧本 `(beat)` 停顿 |

关键陈述：原子 / 场景节拍是 `timeline_scale: "moment"` 的 `TimelineEvent`，可选地配对一个 KnowledgeEntry（双重关注）；结构节拍是 `trait_type: "structural_role"` 的 `BodyAttribute`；排序经由 `precedes` Relations；选择使用带 `timeline_scale: "moment"`、`timeline_event_ids` 或含 `beat` 的 `entry_types` 的 `Scope` 过滤。操作库导出 moment 尺度过滤与节拍表排序辅助（`filterTimelineEventsByMomentScale`、`orderTimelineEventsByIds`、`orderTimelineEventsByPrecedes`）—— 在调用方提供的数组上的纯函数。

## 世界观激活（Lore activation）

**Lore activation（世界观激活）**定义 KnowledgeEntry 在何种触发条件下倾向浮现进组装上下文。它作为可选 `modules` 字段袋（能力标志 `narrative-modules`）上的 `modules.activation` 内部方言存在；匹配、扫描与排序留在产品侧。

| 已发布词汇 | 线上归宿 | 值 |
|------------|----------|-----|
| 触发键 | `modules.activation.keys` | 主激活触发器（别名、名称、短语） |
| 选择性组合 | `modules.activation.secondary_keys` + `.logic` | `and_any`、`and_all`、`not_any`、`not_all` |
| 常开种子 | `modules.activation.constant` | `true` 标记常开种子候选（`keys` 为空时仍有效） |
| 插入提示 | `modules.activation.order` / `.priority` | 排序与打破平局提示 |
| 放置提示 | `modules.activation.position_hint` / `.outlet` | `before_defs`、`after_defs`、`depth`、`outlet` |
| 匹配风格 | `modules.activation.match` | 字面、正则或整词键匹配（风格由产品扫描器定义） |

关键陈述：`body.summary` 与 `AssemblePacket` 条目的 `snippet` 无需触发键即可作为完整 lore 事实阅读（独立片段）；`constant: true` 条目喂给常开种子集，带键条目喂给激活池；lore 邻接经 `Relation` 边扩展。

## 知识包（Knowledge pack）

**Narrative Knowledge Pack（叙事知识包）**是便携 lore 包：一组在叙事主机间流转的有序 KnowledgeEntry、Relation 与可选 SourceAnchor。包使用既有线上原子，外加承载目录元数据的产品传输信封。

| 已发布词汇 | 线上归宿 | 值 |
|------------|----------|-----|
| 包原子 | 既有线上对象 | KnowledgeEntry（lore 节点）、Relation（图边）、SourceAnchor（可选溯源） |
| 目录元数据 | 产品传输信封 | `title`、`version`、`creator`、可选 `description` —— 不是 KE 或 AssemblePacket 上的 `modules.*` |
| 逐条目触发条件 | 每个 KnowledgeEntry 的 `modules.activation` | 包携带激活时随每条目流转 |
| 往返 | `extensions` / `modules` / 开放字符串 | 导入方原样保留未知 namespace、未知模块键与开放字符串词汇 |

关键陈述：包 ≠ AssemblePacket（持久库互换 vs 临时 assemble 输出）；叙事主机在产品侧叠加多个包（世界包、角色包、会话包）—— 按导出顺序导入原子、按 id 合并 Relation 图、合并种子集与池，然后在合并后运行激活 / 作用域 / 预算；叠加策略（优先级、覆盖、软删除）由产品决定。

## Assemble 配方（Assemble recipes）

可选 `AssemblePacket.modules` 字段袋下的两个**包级**伴随方言：每条组装条目偏好注入的位置，以及它被激活的原因。

| 已发布词汇 | 线上归宿 | 值 |
|------------|----------|-----|
| 放置行 | `AssemblePacket.modules.placement[]` | `entry_id` + `position_hint`（`before_defs`、`after_defs`、`depth`、`outlet`），可选 `depth` / `outlet` |
| 激活轨迹行 | `AssemblePacket.modules.activation_trace[]` | `entry_id` + `reason`（`constant`、`key`），带可选匹配键详情 |

关键陈述：数组顺序即互换顺序 —— 主机读取提示后应用自己的布局，条目可省略放置行（主机随后使用本地默认）；按 `entry_id` 把 `placement[]` / `activation_trace[]` 联到 `entries[]`；条目级 `modules.activation` 是偏好的持久创作家园，而包级 `placement[]` 是本包的组装快照；基线 `AssemblePacket` 保持仅线上的精简条目 —— `modules` 经 `narrative-modules` 可选加入。

## 心智状态（Mental state）

**心智状态**定义叙事主机如何在不共享引擎的前提下互换心智状态数据（信念、目标、意图、情绪、观察可达性）。它作为三个 `modules.*` 方言存在，能力标志 `l5-mind` + `narrative-modules`；时序记录对象（`MindState`）承载派生快照/增量。

| 已发布词汇 | 线上归宿 | 值 |
|------------|----------|-----|
| 心智字段 | `KnowledgeEntry.modules.mental` | 九字段词汇：`identity`、`beliefs`、`attention`、`goals`、`intentions`、`emotions`、`dispositions`、`norms`、`constraints` |
| 信念标签 | `KnowledgeEntry.modules.belief` | `holder`（entry_id 或 `world`）、`proposition`、`order`（0–3）、七维标签（Truth/Access/Representation/Content/Source/Context） |
| 事件观察 | `TimelineEvent.modules.observation` | `observers: entry_id[]`、可选 `access`（感知可达性约束） |
| 时序记录 | `MindState` 线上对象 | `mind_state_id`、`holder_entry_id`、`snapshot`、`deltas` —— 严格派生 |

关键陈述：心智字段与信念标签的常驻家园是持有者 KnowledgeEntry 的 `modules.mental`/`modules.belief`；`MindState` 严格时序/派生（无双权威）；假信念 = 真世界事实 + 假角色信念（一行标签记录）；未被观察的事件 ⇒ 陈旧信念 ⇒ 假信念（Knowledge Access 推导链）。

## 相关页面

- [数据模型参考](/zh/reference/data-model) —— 这些词汇所依托的开放字符串字段。
- [核心概念](/zh/explanation/concepts) —— 开放词汇姿态与能力标志。
- [通读 ToyWorld 参考适配器](/zh/how-to/walk-toy-world) —— 已提交 fixture 图中的画像 `entry_type: "beat"`。
