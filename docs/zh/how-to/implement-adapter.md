---
title: 实现 Adapter
---

# 实现 Adapter（Implement an adapter）

你的产品通过在一个 adapter 类型上实现**所声明能力对应的 port 族**、再调用操作库中匹配的 `orchestrate*` 入口来「说 SPOKE」。adapter 是产品存储与 I/O 接触协议的唯一点 —— 每次读写都流经它，而每条协议规则都在持久化之前由库执行。

## 1. 选择你的能力等级

adapter port 类型按能力切片。选择与你的主机所声明能力匹配的组合别名：

| 能力标志 | 需要实现的 ports | 组合别名 |
|----------|------------------|----------|
| `spoke-baseline` | `KnowledgeEntryPort`、`RelationPort`、`ScopeQueryPort`、`FindingPort`、`RuleQueryPort`、`HostManifestPort` | `BaselineAdapter` |
| `spoke-baseline` + `l2-computable` | 基线 + `ComputablePort` | `ComputableAdapter` |
| `spoke-baseline` + `l5-fork` | 基线 + `ForkTimelineQueryPort` | `ForkAdapter` |
| 三者全含 | 完整组合 | `FullAdapter` |

这些别名与 `BaselinePorts` / `ComputablePorts` / `ForkPorts` / `FullPorts` 命名相同的 port 交集。从操作包导入：

```ts
import type {
  BaselineAdapter,
  ComputableAdapter,
  ForkAdapter,
  FullAdapter,
  KnowledgeEntryPort,
  RelationPort,
  ScopeQueryPort,
  FindingPort,
  RuleQueryPort,
  HostManifestPort,
  ComputablePort,
  ForkTimelineQueryPort,
} from "@42ch/spoke-operations";
```

## 2. 实现各 port

port 方法在规范面上是异步的：TypeScript 方法返回 `Promise<SpokeResult<T>>`，Rust trait 声明 `async fn …(&self, …) -> SpokeResult<T>` —— 编排器会 `await` 每一次调用。

### KnowledgeEntryPort —— 条目持久化

```ts
getKnowledgeEntry(entryId: string): Promise<SpokeResult<KnowledgeEntry>>;
putKnowledgeEntry(
  entry: KnowledgeEntry,
  expectedBaseRevision: number | null,
): Promise<SpokeResult<KnowledgeEntry>>;
```

`putKnowledgeEntry` 受乐观并发控制：`expectedBaseRevision: null` 表示条目必须不存在（创建）；非 null 值表示存储中的当前修订号必须等于该值（更新）。否则以 `STORED_REVISION_STALE` 或 `REVISION_CONFLICT` 拒绝。要实现真正的并发安全，请在 adapter 中实现原子 compare-and-put（CAS）—— 库本身保持无 I/O。

### RelationPort —— 关系持久化

```ts
getRelation(relationId: string): Promise<SpokeResult<Relation>>;
putRelation(
  relation: Relation,
  expectedBaseRevision: number | null,
): Promise<SpokeResult<Relation>>;
```

修订号分配由 adapter 负责：创建时（`expectedBaseRevision: null`）初始化 `revision = 1`；接受更新时持久化 `revision = stored + 1`。返回的 Relation 携带已分配的修订号 —— 调用方无需自行设置。

### ScopeQueryPort —— check/assemble 的作用域读取

```ts
listKnowledgeEntries(scope: Scope): Promise<SpokeResult<KnowledgeEntry[]>>;
listTimelineEvents(scope: Scope): Promise<SpokeResult<TimelineEvent[]>>;
```

编排器通过这两个 port 加载作用域数据，再应用作用域过滤辅助函数（`filterKnowledgeEntriesByScope`、`filterTimelineEventsByScope`）收窄到请求的 `entry_ids` / `entry_types` / `timeline_scale` 细化条件。

### FindingPort —— 检查器输出持久化

```ts
putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>>;
```

Finding（检查器输出）在 `orchestrateCheck` 运行完你的检查器回调后，经此 port 持久化。

### RuleQueryPort —— 规则解析

```ts
listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>>;
```

解析 `check` 的规则引用。请求内嵌的 `rules[]` 按 `rule_id` 覆盖；解析失败的引用会使 check 被拒绝。

### HostManifestPort —— 协作元数据

```ts
getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>>;
listPeerHostCapabilityManifests(): Promise<SpokeResult<HostCapabilityManifest[]>>;
```

自身清单与产品已知的对端清单。该 port 为基线必需 —— 它是进程内协作面（主机角色、能力标志、所拥有的 namespaces）。

### ComputablePort —— 可选 `l2-computable`

```ts
project(request: ProjectRequest): Promise<SpokeResult<ProjectResponse>>;
compute(request: ComputeRequest): Promise<SpokeResult<ComputeResponse>>;
```

会话级 computable I/O。`orchestrateProject` / `orchestrateCompute` 先校验请求，再委托给这些方法；在动态边界缺失方法时返回 `CAPABILITY_PORT_MISSING`。

### ForkTimelineQueryPort —— 可选 `l5-fork`

```ts
listForkTimelineEvents(
  scope: Scope & { fork_id: ForkId },
): Promise<SpokeResult<TimelineEvent[]>>;
```

Fork 作用域的时间轴读取。同一个对象可同时满足 `ScopeQueryPort` 与该 port。

## 3. 让 adapter 保持 I/O 边界

操作库相对主机 I/O 是纯函数：存储访问、LLM 调用、排序（ranking）、检索（retrieval）与传输绑定都由你的产品经这些注入 ports 提供。adapter 实现 ports；库执行门禁。参考 `ToyWorldAdapter` 位于 [`fixtures/toy-world/`](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world)，完整讲解见[通读 ToyWorld 参考适配器](/zh/how-to/walk-toy-world)。

## 4. 集成方提示

- **事务边界由 adapter 负责。** 多条目 upsert 与其他多写序列跨越多次 `put*` 调用；原子边界由你的 adapter 决定。
- **Active 唯一性辅助函数接收调用方提供的对等集合。** 编排提供批次内对等集合；当唯一性须覆盖整个存储时，传入存储级快照。
- **动态边界缺失可选 port 时返回 `CAPABILITY_PORT_MISSING`。** `HostManifestPort` 为基线必需，从不被该码门禁。

## 下一步

- [编排操作（orchestrate ops）](/zh/how-to/orchestrate-ops) —— 你的 adapter 解锁的 `orchestrate*` 调用。
- [通读 ToyWorld 参考适配器](/zh/how-to/walk-toy-world) —— TypeScript 与 Rust 两套完整 `FullAdapter`，附一致性测试台。
- [数据模型参考](/zh/reference/data-model) —— 你的 ports 持久化的线上对象。
- [操作线上参考（Ops wire）](/zh/reference/ops) —— 请求/响应信封形状。
