---
title: 历史参考
---

# 历史参考（`spoke-history`）

`spoke-history` 是可选能力：对已存实体做精确修订重放，并显式裁剪保留的检查点。它覆盖 `KnowledgeEntry` 与 `Relation` 数据；检查点存储与重建由提供方拥有。

## 能力（Capability）

| 方面 | 契约 |
|------|------|
| 能力字符串 | `spoke-history` —— `HostCapabilityManifest.capabilities` 的可选成员 |
| manifest 声明 | 主机服务修订快照与历史操作契约 |
| 依赖 | 无共能力；`spoke-baseline` 与 `spoke-knowledge` 既不隐含也不替代它 |
| 协商 | 双方 Connect 对等节点都声明它，该字符串才进入 `negotiated_capabilities`；分派要求它是该协商集的精确成员 |
| Connect 面 | 门控 `history.revisions.get` 与 `history.revisions.prune` |

`capabilities` 是成员唯一的开放字符串数组，因此声明 `spoke-history` 对既有能力词汇是增量的。

`spoke-history` 的精确成员资格准入核心行。请求的 `scope.cognition` 声明了要求时，还须协商 `spoke-cognition`：响应方在探测历史 port 之前拒绝 `capability_missing`，因此单有 `spoke-history` 不能准入带认知要求的请求。

## 操作（Operations）

| `op` | 请求 | 成功值 | 幂等性 |
|------|------|--------|--------|
| `history.revisions.get` | `HistoryRevisionsGetRequest` —— `entity_kind`、`entity_id`、`revision`；可选 `scope`、`extensions` | `RevisionSnapshot` | 只读；无幂等键 |
| `history.revisions.prune` | `HistoryRevisionsPruneRequest` —— `entity_kind`、`entity_id`、`keep`、`idempotency_key`；可选 `scope`、`extensions` | `HistoryPruneResult`，含 `deleted` 与 `floor_revision` | 必需 |

两个操作都走既有的不透明 Connect 载荷与响应信封。`idempotency_key` 含 1–128 个 Unicode 码点；长度按码点计，不按 UTF-16 代码单元或 UTF-8 字节计。

### `RevisionSnapshot`

封闭对象，含以下必填字段：

| 字段 | 类型 | 契约 |
|------|------|------|
| `schema_version` | 共享 `SchemaVersion` | `RevisionSnapshot` 线上 schema 的版本 |
| `entity_kind` | string | 实体种类选择器；核心值为 `knowledge_entry` 与 `relation` |
| `entity_id` | string | 不透明稳定实体 id；对知识条目等于 `snapshot.entry_id`，对关系等于 `snapshot.relation_id` |
| `revision` | integer ≥ 0 | 快照所表示的修订；等于所选实体快照自身的 `revision` |
| `snapshot` | `oneOf`：`KnowledgeEntry` \| `Relation` | 该实体在 `revision` 时的已存状态，已存字段原样保留 |
| `extensions` | 共享 `ExtensionMap` | 产品 namespace 扩展；未知 namespace 与键原样往返 |

`snapshot` 分支与 `entity_kind` 一致。知识条目携带其已存的 `body`、`modules`、`owner`、`disclosure`、`source_anchor`、`status` 与 `extensions`（存在时）；关系携带其既有的关系信封。两个提供方在同一修订返回同一快照时，在 RFC 8785 规范 JSON 下产生逐字节相同的结果。

### `HistoryPruneResult`

| 字段 | 类型 | 契约 |
|------|------|------|
| `deleted` | integer ≥ 0 | 本次调用删除的检查点数；不含活行与派生修订 |
| `floor_revision` | integer ≥ 0 \| null | 当前保留地板：最新被删检查点修订加一；未删除任何检查点时为 `null` |

### 保留地板（Retained floor）

检查点是用于重建历史修订的保留前像。活行不是检查点，也绝不消耗 `keep`。

对单个 `(scope, entity_kind, entity_id)` 目标，`prune_history(..., keep)` 保留最新的 `keep` 个检查点并删除所有更旧的检查点：

- 检查点为 `{1, 3, 5}` 且 `keep = 2` 时，保留集为 `{3, 5}`，删除集为 `{1}`，因此 `deleted = 1`、`floor_revision = 2`。
- 重放修订 1 位于地板之下，以 `details.floor_revision = 2` 拒绝 `HISTORY_TRUNCATED`。
- 重放修订 2 恰好位于地板，由保留检查点 3 应答。
- 未删除任何检查点的裁剪返回 `{ "deleted": 0, "floor_revision": null }`。

重放对位于地板或其上的修订，从位于所请求修订或其上的最近保留检查点应答。位于地板或其上、但因实现声明的保留缺口无法重建的修订，拒绝 `HISTORY_NOT_RETAINED`。

### `HistoryPort`

`HistoryPort` 独立存在，与 `KnowledgeEntryPort` 及 `RelationPort` 分开组合。异步签名为：

```text
get_history_snapshot(principal, scope, entity_kind, entity_id, revision)
  -> SpokeResult<RevisionSnapshot>

prune_history(principal, scope, entity_kind, entity_id, keep)
  -> SpokeResult<HistoryPruneResult>
```

每个方法都以已认证 `Principal` 作为首个准入上下文参数、以必填 `Scope` 作为第二参数，因此实体读写授权绑定到调用方，而非载荷声称的身份。TypeScript 使用 camelCase 命名与 `Promise<SpokeResult<T>>`；Rust 使用 `#[async_trait] async fn` 与上述 snake_case 命名。预期结果以 `SpokeResult` 返回，而非抛出。

Connect 服务面是 `RemoteHistoryService`，经 `HistoryPortService` 适配提供方 `HistoryPort`，与 `RemoteMemoryService` / `MemoryPortService` 同构。服务面拥有对等节点准入、有效作用域解析与裁剪去重。

## 拒绝词汇（Refusal vocabulary）

| 代码 | 条件 | 必填 `details` |
|------|------|----------------|
| `HISTORY_TRUNCATED` | 所请求修订低于当前保留地板，其检查点历史已被丢弃 | `{ "floor_revision": integer ≥ 0 }` |
| `HISTORY_ENTITY_UNSUPPORTED` | Connect 已准入该操作，但主机不服务所请求的 `entity_kind` | `{ "entity_kind": string }`，含所请求的确切种类 |
| `HISTORY_NOT_RETAINED` | 所请求修订位于地板或其上，但因实现声明的保留缺口无法重建 | `{ "retention_reason": string }` —— 提供方给出的可读原因；稳定代码仍是权威 |
| `capability_missing` | `spoke-history` 不在协商集中，或请求声明了 `scope.cognition` 要求而 `spoke-cognition` 不在协商集中；两种拒绝都先于历史服务面探测 | 无；拒绝先于处理方执行且无副作用 |
| `op_unsupported` | `spoke-history` 已协商，但主机未提供服务历史面（`HistoryPort` 服务） | 无；拒绝先于处理方执行且无副作用 |
| `invalid_input` | 请求已协商的 `scope.cognition` 要求不受支持或无效 —— 未知谓词、未实现的视图/维度/指标，或 `recorded_at` 这类不可用的视图；在分派边界以该小写代码拒绝，区别于服务解码器的 `INVALID_INPUT` | `{ "reason": "unsupported_cognition_requirement", "field": string }`；`field` 为载体相对路径，例如 `scope.cognition.view` |
| `INVALID_INPUT` | 请求畸形；裁剪缺 `idempotency_key`；`revision` 或 `keep` 超出接收运行时的精确整数范围 | `{ "op": string }`；解码器诊断放在 `message` 中，调用方按代码分支 |
| `NOT_FOUND` | 实体缺失或不可披露，或 `history.revisions.get` 的请求 `scope` 与对等节点准入作用域不一致 | 共享信封之外无额外成员 |
| 实体授权结果 | 裁剪调用方缺少所选实体的写权限，或裁剪请求 `scope` 与准入作用域不一致 | 实体既有的写授权拒绝 |
| `STORED_REVISION_STALE` | 条件实体 put 的 `expected_base_revision` 不等于当前行修订；历史地板状态不参与 | 实体既有的过期修订细节，例如 `{ "expectedBaseRevision": 1, "storeRevision": 5 }` |
| `IDEMPOTENCY_CONFLICT` | 裁剪重试复用同一键但载荷摘要不同 | 无 |
| `RECEIPT_EXPIRED` | 裁剪重试重放其记录结果已被逐出的键 | 无 |
| `INTERNAL_ERROR` | serve 预算为零，响应方在调用历史服务之前拒绝；提供方从未被调用 | `{ "kind": "timeout" }` |
| `OUTCOME_UNCERTAIN` | 非零 serve 等待在调用后超出本地预算，或调用后服务失败；提供方结果未知 | `{ "retry_class": "uncertain", "kind": "timeout" \| "panic" }` |

`HISTORY_TRUNCATED`、`HISTORY_ENTITY_UNSUPPORTED` 与 `HISTORY_NOT_RETAINED` 是新增的 `SpokeRejectCode` 成员；其余代码保持既有含义。

## 使用约束（Usage constraints）

- **重放即实体数据。** 快照原样返回实体在 `revision` 时的已存状态；重放不重新推导认知或其它派生状态。已存的 `modules.cognition` 值作为实体数据传递，`MindState` 等派生视图保持自己的拥有者。
- **字节存在性保留。** 显式存在且为空的已存成员 —— 包括 `body.tags: []`、`body.state: {}`、`modules: {}`、`body.attributes: []` 与 map 形状的 `body.computable` —— 在重放与 RFC 8785 规范化后仍保留成员名与空值，逐字节不变。
- **实体种类开放。** 核心值为 `knowledge_entry` 与 `relation`。服务额外种类时，提供方须声明其编码已存实体所用的核心形状（`KnowledgeEntry` 或 `Relation`）并一致使用该 `snapshot` 分支；否则应答 `HISTORY_ENTITY_UNSUPPORTED`。未知种类绝不路由到核心实体 port。
- **裁剪是显式的。** `history.revisions.prune` 是删除检查点的唯一路径；任何写路径都不隐式裁剪。`keep` 统计保留的检查点数，不含活行。
- **裁剪使用实体写权限。** 裁剪要求与同一作用域内对所选实体 put 完全相同的权限。读取遵循所选实体的读取与披露规则。不存在单独的 `history_admin` 授权，也没有历史专用授权代码。
- **幂等由 connect 拥有。** 服务面按 `(effective_scope, requester_principal_id, op, idempotency_key)` 记录 `{payload_digest, outcome}`，其中 `payload_digest` 是请求载荷 RFC 8785 规范 JSON 的 SHA-256。同键同摘要重放返回记录结果且不重新执行 port 调用；记录下来的响应校验失败以该错误重放；摘要不同应答 `IDEMPOTENCY_CONFLICT`；记录被逐出应答 `RECEIPT_EXPIRED` 且绝不重新执行。去重记录比 Connect 会话存活更久，被逐出记录留下的墓碑保留至存储生命周期结束。
- **OCC 不变。** 条件实体 put 将 `expected_base_revision` 与当前行修订比较；历史不增加并行的写冲突拒绝，地板状态与保留历史都不准入或拒绝写入。
- **整数范围精确。** `revision` 与 `keep` 具有 `u64` 语义且无 schema 上限。无法精确表示所接收值的运行时在解码时以 `INVALID_INPUT` 拒绝，绝不取整、截断或强制转换。
- **作用域是准入结果，不是断言。** 请求 `scope` 可选；存在时必须等于准入作用域。不一致时，get 应答 `NOT_FOUND`，prune 应答所选实体既有的写授权拒绝，且不调用 port。
- **成功值在两端解码边界校验。** 服务面在发出成功前校验提供方输出，每个 `RemoteAdapter` 在返回 `Ok` 前校验响应；畸形成功值产生传输解码错误，绝不产生 `Ok`。

## 相关页面

- [协议参考](/zh/reference/protocol) —— 能力标志与 schema 清单。
- [操作线上（Ops wire）参考](/zh/reference/ops) —— 请求/响应信封、`Scope` 与 `ErrorEnvelope`。
- [connect 参考](/zh/reference/connect) —— 承载 `history.*` 载荷的信封与分派门禁。
- [数据模型参考](/zh/reference/data-model) —— `KnowledgeEntry`、`Relation` 与 `ExtensionMap` 字段表。
