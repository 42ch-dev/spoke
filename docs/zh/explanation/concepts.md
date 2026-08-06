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
| **L5 时间** | TimelineEvent when 轴 + 投影层级 `brief` / `narrative` / `moment`（`l5-fork` 下可选 Fork） |
| **L6 约束** | Rule 声明式 `check` 输入 |
| **L7 Finding** | 检查器输出 + 状态生命周期 |
| **L8 上下文** | AssemblePacket 线上形状（组装本身留在产品侧） |

## 能力等级

产品按声明的能力等级主张合规。**`spoke-baseline`** 经五个 ops 线上族、`HostCapabilityManifest` + 基线 `HostManifestPort`、以及共享 `Scope` / `error-envelope` 定义覆盖 L0–L8 语义。可选标志是增量的 —— 基线合规可独立成立：

- **`l2-computable`** —— `body.state` / `body.computable`、`TimelineEvent.computable_logs`，以及 `project` / `compute` ops。
- **`l5-fork`** —— TimelineEvent 上的 `fork_id` / `parent_fork_id` 分支元数据与 `Scope.fork_id` 过滤。
- **`narrative-modules`** —— 面向跨产品功能方言的可选 `modules`（`ModuleMap`）字段袋。
- **`spoke-connect`** —— 可选交互信封族；讲该协议的主机在 `HostCapabilityManifest.capabilities` 中列出该标志。

connect 家族的会话生命周期、信封认证与能力路由在 [Connect 架构](/zh/explanation/connect) 中说明。

## 双重关注（dual-concern）对

协议刻意分开两对概念：

- KnowledgeEntry 上的本体 `entry_type: "event"` 标签与 L5 `TimelineEvent` when 轴对象 —— 一个本地概念可映射到其中一类或两类线上形状。
- 本体 `entry_type: "rule"` 标签与 L6 `Rule` 检查器输入 —— Rule 是声明式输入，Finding 是检查器输出。

名称保持分离，使 `check` / `assemble` 选择器保持无歧义。

## 开放词汇与 Domain Profile

核心对象保持封闭信封（`additionalProperties: false`），而核心词汇保持**开放**：`entry_type`、`relation_type` 与 statuses 都是带记录在案核心列表的开放字符串，而非封闭枚举。Domain Profile 在这些开放字符串之上发布本体词汇 —— beat 映射、lore 激活、知识包 —— 产品以开放字符串与 `extensions.<namespace>` 表达画像专属类型。见[领域画像（Domain Profiles）](/zh/explanation/domain-profiles)。

## 选择器与扩展点

- **Scope** —— `check` / `assemble` 的共享 ops 选择器：必填不透明 `scope_id` 加可选细化（`entry_ids`、`entry_types`、`timeline_scale`、`fork_id` 等）。
- **Extensions** —— 每个持久对象上的 `extensions.<namespace>` 产品字段袋；adapter 原样往返未知 namespace。
- **Modules** —— KnowledgeEntry 与 AssemblePacket 上的可选 `modules.*` 字段袋（能力标志 `narrative-modules`），用于跨产品功能方言。

## 纯函数库姿态

操作库是生成线上类型之上的纯行为层：生命周期辅助、按能力切片的 adapter ports 与注入式编排 —— 自身无任何 I/O。存储访问、LLM 调用、ranking、retrieval 与传输绑定由产品经注入 ports 提供；`fixtures/toy-world/` 中的参考 adapter 演示了这一模式。

## 相关页面

- [协议参考](/zh/reference/protocol) —— schema 清单、扩展契约、能力标志。
- [数据模型参考](/zh/reference/data-model) —— 每层对应的字段表。
- [领域画像（Domain Profiles）](/zh/explanation/domain-profiles) —— 已发布的开放字符串词汇。
