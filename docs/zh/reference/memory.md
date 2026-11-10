---
title: 记忆（Memory）
---

# 记忆（Memory，`spoke-memory`）

以线上族（wire family）形式提供的按主体长期记忆：空间、主体、授权、捕获、候选审核、保留记录、召回、作者画像、反思，以及带版本的导出/导入。记忆是**主观的** —— 主机为某一主体捕获的印象 —— 它作为独立的族与既定的 KnowledgeEntry 数据并列存在。每个持久记忆对象的必填字段：`schema_version`、`extensions`。

该族是可选能力。主机通过在 `HostCapabilityManifest.capabilities` 中声明 `spoke-memory` 来认领它，线上契约即下面十三个模型加 33 个 `memory.*` 操作。

## 该族承载什么

| 关注点 | 线上产物 |
|--------|----------|
| 记忆存放位置 | `MemorySpace` —— 每个空间一个权威提供方，含 `revision` 与计算的 `pending_receipts` |
| 记忆关于谁 | `Subject` —— 稳定不透明 `subject_id` 加可选带命名空间的 `kind`；扁平，一条记录一个主体 |
| 谁可以做什么 | `MemoryGrant` —— 能力集 + 可选 subject 范围 + 可选 `context_ids` + `shared_access` + 到期时间 |
| 捕获与审核 | `memory.capture` → `MemoryCandidate`（不可变的 `capture_scope`、咨询性 `classification`）→ `memory.candidates.review` / `.dismiss` |
| 保留记忆 | `RetainedRecord` —— 不受信 `content`（`text` + 可选不透明 `structured` + 可选记录时 `cognition`）、放置 `scope`、来源、`revision`、可选 `tombstone` |
| 读取记忆 | `ReadSelection`（放置模式）+ `ProvenanceFilter`（来源上下文投影）+ 可选 `cognition_filter` + `memory.recall` 预算与截断 |
| 作者基准 | `ProfileDocument` —— 受信的作者维护内容种类（`personality`、`preferences`） |
| 派生综合 | `Reflection` —— 基于固定基准生成，`stale` 在读取时计算 |
| 操作结果 | `MemoryReceipt` —— 持久、载荷绑定、无内容（`committed` / `pending` / `failed`） |
| 提供方替换 | `MemoryExportBundle` + `MemoryExportChunk`，以及分阶段导入激活流程 |

## 能力标志

| 标志 | 门控 | 操作 |
|------|------|------|
| `spoke-memory` | 族核心 | 25 个核心 `memory.*` 操作 |
| `spoke-memory-reflection` | 反思生成、状态、列举 | `memory.reflections.state`、`.list`、`.generate` |
| `spoke-memory-transfer` | 导出/导入 | `memory.exports.create`、`.get`，`memory.imports.stage`、`.activate`、`.discard` |

声明子标志的主机同时声明 `spoke-memory`；仅提供 `spoke-memory` 的主机在任何处理器运行之前以 `capability_missing` 拒绝子标志的操作。标志在 connect 会话上逐个协商，dispatch 门只评估协商结果集。

## 操作

| 操作 | 意图 |
|------|------|
| `memory.describe` | 提供方自描述：协议版本、实际提供的能力标志、限额、必需画像种类、引擎策略版本 |
| `memory.spaces.list` | 调用方可见的空间，含 `pending_receipts`（排水屏障读取） |
| `memory.spaces.freeze` | 为维护或迁移冻结空间 |
| `memory.spaces.unfreeze` | 重新打开已冻结空间 |
| `memory.subjects.ensure` | 幂等地获取或创建主体 |
| `memory.subjects.get` | 单个主体记录 |
| `memory.subjects.fence` | 围栏、解围栏或归档主体；围栏或解围栏某个放置上下文 |
| `memory.grants.list` | 空间内的授权（管理员视图或调用方自身） |
| `memory.grants.grant` | 在签发者自身覆盖范围内签发授权 |
| `memory.grants.revoke` | 终止性撤销授权 |
| `memory.capture` | 将原始材料捕获为待审候选 |
| `memory.candidates.list` | 待审与已消耗候选 |
| `memory.candidates.get` | 单个候选 |
| `memory.candidates.dismiss` | 不做出审核决定即消耗候选 |
| `memory.candidates.review` | 提升、拆分或丢弃 —— 唯一的候选 → 记录路径 |
| `memory.records.create` | 直接作者写入 |
| `memory.records.get` | 单条保留记录，重新鉴权 |
| `memory.records.update` | 更新内容或 kind（放置变更使用 `promote`） |
| `memory.records.delete` | 移除内容，保留墓碑 |
| `memory.records.list` | 选择 + 过滤列举，含 `total_authorized` |
| `memory.records.promote` | 在上下文之间或向共享放置做显式变更 |
| `memory.recall` | 经授权、排序、受预算约束的执行期读取 |
| `memory.profile.get` | 作者画像文档（缺失种类为诚实空结果） |
| `memory.profile.update` | 创建或更新作者画像文档 |
| `memory.receipts.get` | 轮询回执，解决不确定结果 |
| `memory.reflections.state` | 当前基准摘要、陈旧性、数据充分性、缺失的必需种类 |
| `memory.reflections.list` | 反思列表，最新在前，陈旧性在读取时计算 |
| `memory.reflections.generate` | 基于固定基准的显式、计费生成 |
| `memory.exports.create` | 产出分块的逻辑导出 |
| `memory.exports.get` | 获取单个导出分块 |
| `memory.imports.stage` | 按严格序数暂存分块，并在末块执行校验 |
| `memory.imports.activate` | 原子可见性切换；递增 `authority_generation` |
| `memory.imports.discard` | 激活前回滚暂存数据 |

读操作返回其载荷；每个变更操作返回 `MemoryReceipt`。只有反思生成、导出创建、暂存末块与导入激活可能报告 `pending`，完成情况通过 `memory.receipts.get` 观察。

## 认知内容与过滤（`spoke-cognition`）

`Content.cognition` 是完整、封闭的标准认知记录的可选数组。每个条目携带 `view: "recorded_at"`，并保留自己的归因、基准与时间；缺失表示未记录任何标准认知 —— 成员缺省绝不会被序列化为 `[]`，而显式出现的空数组是合法值，作为独立线上值保留，既非缺省所产生，也不与非空数组等价 —— 墓碑化内容仅为 `{ "text": "" }`，不含 `structured` 或 `cognition` 成员。原始文本、`structured` 或候选材料绝不会仅因放置而升格为完成的标准认知。

已记录认知按内容校验，而不是按授权校验：每个存在的成员都必须是 `view: "recorded_at"` 的封闭记录。该检查在 `memory.capture` 以及 `memory.records.create` / `.update` 的字节准入检查之前运行；畸形成员以操作层 `INVALID_INPUT` 在 `content.cognition` 上拒绝请求，并带 `details.reason: "invalid_cognition_record"`，这与不受支持的认知需求所用的分发层 `invalid_input` 线上码不同。非空认知数组计入捕获分类；既有内容字节预算计量每个显式存在的成员而非将其丢弃 —— `[]` 携带自己的准入字节。候选审核默认使用捕获时的提议内容：`promote` 决定可提供新的 `content_override`，`fragment` 决策携带各片段的新 `content`，两者都按同一 recorded-at-time 规则在任何状态变更前独立校验，且覆盖内容优先于提议内容。审核仍要求覆盖候选 `capture_scope` 的既有 `review` 与 `records_write` 授权、逐字继承该作用域，且待审不授予任何权限。

`cognition_filter`（`CognitionFilter`）在恰好五个请求上被接受 —— `memory.recall`、`memory.records.list`、`memory.reflections.state`、`memory.reflections.list` 与 `memory.reflections.generate` —— 并要求已协商的 `spoke-cognition` 标志。它是针对记录时内容的硬过滤：授权先行，既有授权覆盖仍然门控该调用；在合格集合之上，仅当某记录的至少一条 `Content.cognition` 条目满足完整合取时才匹配。它与 `selection` 及来源 `filter` 以 AND 组合，绑定游标身份（任何状态或授权变化以 `CURSOR_INVALID` 使游标失效），并在计数、排序、分页限额与字节预算之前应用。`memory.records.list` 在该留存集合上报告 `total_authorized`，`memory.recall` 只在其内排序与截断。该过滤既不扩大也不替代既有授权，也不增加 OR、文本相似、排序输入或默认截断。

三个反思请求接受同一过滤器并将其固定。`memory.reflections.generate` 将其写入 `Reflection.cognition_filter` 并纳入反思基准：基准身份覆盖该过滤器以及已授权记录 id 与 revision 集合、画像 revision 与引擎策略身份，因此过滤器或来源集变化会改变 `state_digest`，反思随即读为陈旧。`memory.reflections.list` 按固定过滤器匹配，`memory.reflections.state` 将其纳入返回的摘要。缺省即没有固定过滤器。

未知谓词或未实现的维度会在分发边界以 `invalid_input` 线上码加 `details.reason: "unsupported_cognition_requirement"` 拒绝整个请求，与[认知选择](/zh/reference/ops#cognition-selection-spoke-cognition)一致。线上形状见[数据模型参考 §认知](/zh/reference/data-model#cognition-spoke-cognition)。

## 数据模型

| 模型 | 角色 |
|------|------|
| `MemorySpace` | 单一权威存储；冻结、导出与导入激活的单位 |
| `Subject` | 记忆所关于的实体；`active` / `fenced` / `archived` |
| `Principal` | 由准入口径建立的已认证调用方身份 |
| `MemoryGrant` | 提供方强制执行的授权记录 |
| `RetainedRecord` | 已审核或直接写入的持久记忆 |
| `MemoryCandidate` | 等待审核的暂定捕获 |
| `ProfileDocument` | 受信的作者人格/偏好基准 |
| `Reflection` | 基于固定基准的派生综合 |
| `MemoryReceipt` | 无内容的操作回执（唯一异步模型） |
| `ReadSelection` | 读取的放置选择模式 |
| `ProvenanceFilter` | 来源上下文投影过滤器，与放置相互独立 |
| `MemoryExportBundle` | 带版本与摘要的逻辑导出 |
| `MemoryExportChunk` | 导出的有界线上单元 |

## 提供方与 adapter 指引

`MemoryPort` 是独立的可选边界 —— TypeScript 接口与 Rust trait，每个 `memory.*` 操作对应一个类型化方法，已认证主体显式传入。示意形状（请求/响应类型即生成的 `memory-*` 线上类型）：

```ts
import type { MemoryPort } from "@42ch/spoke-operations";
import type { Principal } from "@42ch/spoke-schemas";

const port: MemoryPort = {
  describe(principal: Principal, request: MemoryDescribeRequest) { /* … */ },
  capture(principal: Principal, request: MemoryCaptureRequest) { /* … */ },
  // … 33 个类型化方法，每个对应一个 memory.* 操作
};
```

- **可用性探测。** `MemoryPorts.asMemory()`（TS）/ `MemoryPorts::as_memory()`（Rust）报告 adapter 是否提供该族。`MemoryPort` 与基线 port 族并列，是独立的可选接口：提供记忆的产品实现它，其余产品保持该探测为空实现。
- **提供方职责留在提供方。** 授权与非放大校验、`revision` CAS、以 `(space_id, principal, op, idempotency_key)` 为键的持久幂等、回执与去重墓碑、空间冻结与主体/上下文围栏、候选审核原子性，全部位于提供方自己的事务层。
- **通过 connect 提供服务。** 服务侧主机暴露 connect 自有的 `RemoteMemoryService` 接缝；`MemoryPortService` 在提供方 `MemoryPort` 之上实现它，并带一个把已认证 peer 映射为 `Principal` 的准入口径。一个可调用点服务整个族，33 个类型化操作构成解码面。
- **算法策略可分离。** `fixtures/memory-engine/rust` 处的 `spoke-fixture-memory-engine` Rust crate 是仓库内未发布的 fixture，承载确定性的无 I/O 策略（候选分类、召回排序与预算、反思组装与陈旧性判定、传输分块/摘要/映射）。产品不依赖它：提供方直接实现 `MemoryPort`，可以运行该 fixture 引擎的策略、自有策略或完全不运行，仍保持线上兼容。
- **画像是作者的，记录是捕获的。** 画像内容为受信作者材料，其本身不授予任何授权；记录与候选内容是不受信数据。只有当提供方声明的 `required_profile_kinds` 齐备时才允许生成反思，缺失非必需种类则返回诚实空结果。

## 字段表与规格

线上级细节位于 spec 语料（SSOT）；本页通过链接指向它，而不重述：

- [记忆族规格](https://github.com/42ch-dev/spoke/blob/main/.agents/specs/memory/spoke-memory.md) —— 能力标志、不变量 `MEM-R001`–`MEM-R027`、数据模型字段表、逐操作契约、错误与重试类别、线上映射、ProfileDocument 实质内容。
- [记忆能力 ADR](https://github.com/42ch-dev/spoke/blob/main/.agents/adr/implemented/feature/2026-10-29-spoke-memory-family.md) —— 可选族决策、单一权威模型、回执异步模型、引擎/提供方分工。
- [记忆数据 schema](https://github.com/42ch-dev/spoke/tree/main/schemas/data) 与 [记忆操作 schema](https://github.com/42ch-dev/spoke/tree/main/schemas/ops) —— 已提交的线上真源。
- [connect 参考](/zh/reference/connect) —— 承载 `memory.*` 载荷的信封。

## 相关页面

- [协议参考](/zh/reference/protocol) —— 三列模型、能力标志与 schema 清单。
- [操作线上参考](/zh/reference/ops) —— 基线与可选操作族、`ErrorEnvelope`、共享规则。
- [认知记录](/zh/reference/data-model#cognition-spoke-cognition) —— `Content.cognition` 承载的标准记录、过滤语法与反思基准。
- [核心概念](/zh/explanation/concepts) —— 记忆如何与既定知识并列。
- [实现 Adapter](/zh/how-to/implement-adapter) —— 按能力划分的 port 族，含可选记忆提供方。
