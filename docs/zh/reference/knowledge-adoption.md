---
title: 知识采纳参考
---

# 知识采纳参考（`spoke-knowledge-adoption`）

`spoke-knowledge-adoption` 是可选能力：把共享 `KnowledgeEntry` 采纳到目标作用域 —— 一次原子、写一次的复制，带目标方治理，以及记录终态来源的不可变 `adoption_anchor`。它定义一个 Connect 操作 `knowledge.adopt`。

## 能力（Capability）

| 方面 | 契约 |
|------|------|
| 能力字符串 | `spoke-knowledge-adoption` —— `HostCapabilityManifest.capabilities` 的可选成员 |
| manifest 声明 | 主机服务采纳操作与 `AdoptionAnchor` 契约 |
| 共能力 | 声明 `spoke-knowledge-adoption` 必须同时声明 `spoke-knowledge` |
| 协商 | 双方对等节点上，两个精确字符串都必须是 `negotiated_capabilities` 的成员；`spoke-baseline` 或其中任一能力单独都不满足该门禁 |
| Connect 面 | 与 `spoke-knowledge` 一起门控 `knowledge.adopt` |

`knowledge.*` 是核心 Connect 分派决策拥有的保留产品前缀。`knowledge.adopt` 是其唯一已注册操作；其它任何 `knowledge.*` 字符串都是未知保留名。

## 操作（Operations）

| `op` | 所需协商能力 | 请求 | 成功值 | 幂等性 |
|------|--------------|------|--------|--------|
| `knowledge.adopt` | `spoke-knowledge-adoption` 与 `spoke-knowledge` | `KnowledgeAdoptRequest` | `KnowledgeAdoptResponse` 成功分支，含 `outcome` 与 `entry` | 必需 |

该操作走既有的不透明 Connect 请求与响应信封。服务把 `knowledge.*` 保留前缀路由到可选 `RemoteAdoptionService` 面，其 `AdoptionPortService` 适配提供方 `AdoptionPort`；能力缺失时，操作在处理方运行前被拒绝且无副作用。

### `KnowledgeAdoptRequest`

封闭对象：

| 字段 | 存在性 | 类型 | 契约 |
|------|--------|------|------|
| `source_scope_id` | 必填 | 非空字符串 | 所请求的源作用域；退役解析可改变锚点记录的终态作用域 |
| `source_entry_id` | 必填 | 非空字符串 | 所请求的源条目 |
| `source_revision` | 必填 | integer ≥ 0 | 所请求的源修订 |
| `destination_scope_id` | 必填 | 非空字符串 | 主机准入的目标作用域 |
| `destination_entry_id` | 可选 | 非空字符串 | 目标 id；缺省时默认取解析后的终态 `source_entry_id` |
| `idempotency_key` | 必填 | 字符串，1–128 个 Unicode 码点 | Connect 重试键；长度不按 UTF-16 代码单元或 UTF-8 字节计 |
| `extensions` | 可选 | 共享 `ExtensionMap` | 产品请求扩展；显式为空的 `extensions: {}` 在解码与摘要中保持为存在成员 |

### `KnowledgeAdoptResponse`

标准 ops `oneOf` 成功/失败信封。成功分支携带 `outcome` 与 `entry`；错误分支携带 `error`；`extensions` 是两个分支上的可选成员。除 `outcome`、`entry`、`error` 与 `extensions` 外不接受其它成员。

| 成员 | 契约 |
|------|------|
| `outcome` | 新写入副本为 `"adopted"`；重复规则返回的既有副本为 `"already_present"` |
| `entry` | 目标 `KnowledgeEntry`，始终携带格式良好的 `adoption_anchor`；其 `entry_id` 即目标 id —— 提供的 `destination_entry_id`，缺省时为解析后的终态 `source_entry_id` |

提供方结果若条目缺少锚点，或锚点不符合 `AdoptionAnchor` schema，则作为传输/服务拒绝应答，绝不为 `Ok`。

### `AdoptionAnchor`

记录目标条目采纳溯源的封闭对象：

| 字段 | 类型 | 契约 |
|------|------|------|
| `schema_version` | 共享 `SchemaVersion` | `AdoptionAnchor` 线上 schema 的版本 |
| `source_scope_id` | 非空字符串 | 解析后终态源的作用域 id |
| `source_entry_id` | 非空字符串 | 解析后终态源的条目 id |
| `source_revision` | integer ≥ 0 | 解析后终态源的修订 |
| `extensions` | 共享 `ExtensionMap` | 产品扩展；未知 namespace 与键原样往返 |

`KnowledgeEntry.adoption_anchor` 是指向该 schema 的可选 `$ref`，作为条目载荷经 get、put 与服务路径传递。

### 终态来源溯源

存活的可请求源被直接采纳。状态为 `merged` 的源经其退役链解析到终态存活源，采纳复制终态，绝不复制已退役中间节点。锚点记录终态存活源的 `source_scope_id`、`source_entry_id` 与 `source_revision`；默认目标 id 即该终态 `source_entry_id`。

解析在请求条目之后最多沿 32 条退役边（`RETIREMENT_CHAIN_MAX_EDGES`），并追踪每个访问过的条目 id。恰在 32 条边上到达存活终态的链成功。

拒绝时以 `SOURCE_RETIRED` 应答，`details.retirement_chain` 为源条目 id 的有序数组，从请求 id 开始。数组形状取决于拒绝类型：

- **环** —— 闭环的那个 id 在拒绝之前已被访问，因此该重复 id 会再次作为最后一个元素列出：`ke_a → ke_b → ke_c → ke_a` 报告 `["ke_a","ke_b","ke_c","ke_a"]`。
- **其它所有拒绝** —— `merged` 条目零个或多个后继、后继解析到缺失或不可解析条目、终态条目状态为 `deleted`，或需要第 33 条边 —— 只列出已解析的 id：未访问的后继绝不列出，因此数组在最后解析到的节点结束。

### 复制规则

| 方面 | 规则 |
|------|------|
| 保留 | `body`、`canonical_name`、`entry_type` |
| 模块 | `modules` 保留，但 `modules.cognition` 不复制 |
| 溯源字段 | `source_anchor` 被清空；采纳溯源存放于 `adoption_anchor` |
| 扩展 | 原样复制，但丢弃历史遗留的 `mindcore_adoption` namespace；其它每个 namespace 与键都保留，包括显式为空的 namespace 对象 |
| 治理 | `owner` 与 `disclosure` 重置为目标自身值 |
| 目标 id | 提供 `destination_entry_id` 时用其值，否则用解析后的终态源条目 id |
| 目标写入 | 使用 `expected_base_revision: null` 的 create-only OCC；源修订绝不复用为目标已存 OCC 基 |

Port 形状：TypeScript 的 `AdoptionPort.adoptKnowledgeEntry(principal, scope, request)` 与 Rust 的 `adopt_knowledge_entry(&self, principal, scope, request)`，返回 `SpokeResult<KnowledgeAdoptResponse>`。port 接收完整类型化请求，其中 `idempotency_key` 作为透传上下文；去重由 Connect 拥有。

### 重复结果

重复比较子（comparator）是解析后的源元组 `(source_entry_id, resolved_terminal_revision)` —— 与创建写入记录到 `adoption_anchor` 的元组相同。

- 把同一解析元组重复采纳到同一 `destination_scope_id`，返回既有副本且 `outcome: "already_present"`，不写第二份副本。
- 重复时解析后的终态修订与既有副本锚点记录的修订不同，拒绝 `REVISION_CONFLICT`，`details.adopted_revision` 等于所记录的修订。既有副本不被刷新或覆盖。
- 若另一写入方先占用目标槽位，已存条目按 `adoption_anchor` 用同一元组分类：元组相同则返回该副本为 `already_present`；解析终态修订不同则拒绝 `REVISION_CONFLICT`；无关占用者保留 create-only put 拒绝且不被覆盖。

### Connect 幂等性

Connect 在准入的 port 调用之前拥有去重，采用 `MemoryPort` 模型：

- 重试身份是有序数组 `(op, effective_scope, requester_principal_id, idempotency_key)` 的 RFC 8785 规范 JSON 序列化；不同元组在两个语言栈与重连之间始终推导出不同身份。
- 该身份下的记录携带 `payload_digest` —— 完整请求载荷对象 RFC 8785 规范 JSON 的 SHA-256 摘要，保留成员存在性。
- 同一重试身份且 `payload_digest` 相同，返回记录结果，且不发生第二次提供方调用或写入。记录的成功以 `outcome: "already_present"` 重放；记录的拒绝按该拒绝原样重放。
- 同一重试身份但 `payload_digest` 不同，拒绝 `IDEMPOTENCY_CONFLICT`。
- 单飞由去重存储接口拥有：一个存储由多个服务实例共享时，每个重试身份最多一次在途提供方调用；未以已结算结果结束的提供方调用释放其认领，下一次调用重新调用提供方。
- 去重记录比 Connect 会话存活更久；记录被逐出时应答 `RECEIPT_EXPIRED`，且不重新执行采纳。

## 拒绝词汇（Refusal vocabulary）

| 代码 | 条件 | 精确 `details` |
|------|------|----------------|
| `ADOPTION_ANCHOR_IMMUTABLE` | 普通 put 试图更改或移除目标既有的 `adoption_anchor` | 恰为 `entry_id`，即目标条目 id |
| `SOURCE_RETIRED` | 退役解析遇到环、歧义、已删除终态，或需要第 33 条退役边 | 恰为 `retirement_chain`，即有序的已访问源条目 id 数组 |
| `REVISION_CONFLICT` | 既有目标副本锚定了不同的源修订 | 恰为 `adopted_revision`，即既有锚点中的整数修订 |
| `REVISION_CONFLICT` | create-only 目标 put 遇到无法按 `adoption_anchor` 分类的无关占用者 | 无 `adopted_revision` 成员；即 create-only put 拒绝 |
| `NOT_FOUND` | 所请求源缺失，或所请求/解析后的终态源在准入作用域之外，或对另一视角 owner-private | 不含任何标识源的成员；同一通用缺失 |
| `NOT_AUTHORIZED` | 请求 `destination_scope_id` 与准入的 `Scope.scope_id` 不同；该检查先于源链解析 | 无采纳专用成员；源保持不变 |
| `IDEMPOTENCY_CONFLICT` | Connect 请求复用幂等键但请求摘要不同 | 无采纳专用成员 |
| `INVALID_INPUT` | 载荷畸形，包括请求 `extensions` 无效 | 无结构化 `details`；解码器诊断放在 `message` 中，调用方按代码分支 |
| `RECEIPT_EXPIRED` | 该重试身份的去重记录已被逐出 | 共享信封之外无额外成员 |
| `OUTCOME_UNCERTAIN` | 服务等待超出本地预算，或调用后发生服务失败；提供方结果未知 | `retry_class: "uncertain"`、`kind: "timeout"` \| `"panic"` |
| `capability_missing` | `spoke-knowledge-adoption` 或 `spoke-knowledge` 不在协商集中 | 无；拒绝先于处理方执行且无副作用 |
| `op_unsupported` | 两个能力字符串都已协商，但主机未提供采纳服务面 | 无；拒绝先于处理方执行且无副作用 |

`ADOPTION_ANCHOR_IMMUTABLE` 与 `SOURCE_RETIRED` 是采纳族的 `SpokeRejectCode` 成员；`REVISION_CONFLICT`、`NOT_FOUND`、`NOT_AUTHORIZED`、`IDEMPOTENCY_CONFLICT` 与 `INVALID_INPUT` 保持既有代码与含义。`OUTCOME_UNCERTAIN` 报告未知结果：提供方工作既不被取消也不回滚，调用方通过重放同一幂等键恢复。

## 使用约束（Usage constraints）

- **一次原子写入。** 完整采纳是一个事务：失败的采纳既不留下目标副本，也不留下部分锚点，且观察不到任何部分治理重置。create-only 目标 CAS 是并发守卫。
- **采纳是写一次的。** 已采纳副本写一次后，绝不被采纳刷新、同步或重新锚定。后续重复返回既有副本或拒绝 `REVISION_CONFLICT`。
- **锚点不可变。** 每个存储 `KnowledgeEntry` 的 spoke-operations 知识写编排，都逐字节保留已存 `adoption_anchor`，否则在写入前拒绝 `ADOPTION_ANCHOR_IMMUTABLE`。门禁点是 `orchestrateUpsert` / `orchestrate_upsert`、`orchestratePromote` / `orchestrate_promote`，以及服务 `knowledge.put` 的作用域 put 路径；提供方 port 是原子的兜底。
- **`source_anchor` 是独立溯源。** 在目标条目上，`source_anchor` 是独立的溯源字段，并在已采纳副本上被清空。产品专用的 source-anchor 编解码器不是采纳编码；采纳溯源只存在于 `adoption_anchor`。
- **目标治理优先。** 目标 `owner` 与 `disclosure` 值治理新副本；源治理值绝不被继承。`modules.cognition` 绝不跨越。
- **缺失是通用的。** 加载失败的所请求源、准入作用域之外的解析终态，以及对另一视角 owner-private 的条目，都产生同一 `NOT_FOUND`，不含标识源的细节，也不发生目标写入。重复调用产生同一缺失。
- **准入先于解析。** 目标准入在源链解析开始前比较 `destination_scope_id` 与准入的 `Scope.scope_id`；作用域外的目标拒绝 `NOT_AUTHORIZED`，不触碰源。
- **保留前缀。** `knowledge.adopt` 是唯一已注册的 `knowledge.*` 操作。其它任何 `knowledge.*` 字符串在查询任何产品自定义能力映射之前以 `op_unsupported` 拒绝；`ConnectConfig.op_capability_requirements` 与组合响应方等价物都无法注册或授权未知保留名。
- **载荷作用域 id 是选择器。** 调用方必须经主机准入上下文同时准入所请求源与目标作用域；请求作用域 id 是对该准入的校验，绝不是权威。
- **成功值在边界校验。** 提供方成功值在发出前校验；畸形的提供方结果应答 `INTERNAL_ERROR` 传输拒绝，绝不为 `Ok`。

## 相关页面

- [协议参考](/zh/reference/protocol) —— 能力标志与 schema 清单。
- [数据模型参考](/zh/reference/data-model) —— `KnowledgeEntry` 字段，包括 `adoption_anchor`。
- [操作线上（Ops wire）参考](/zh/reference/ops) —— 请求/响应信封、`Scope` 与 `ErrorEnvelope`。
- [connect 参考](/zh/reference/connect) —— 保留分派、能力协商与 invoke 信封。
- [历史参考](/zh/reference/history) —— 采用相同 principal 先行 port 形状的姊妹可选族。
