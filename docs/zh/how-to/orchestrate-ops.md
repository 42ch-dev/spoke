---
title: 编排操作（Orchestrate operations）
---

# 编排操作（Orchestrate operations）

操作库为每个 op 族暴露**一个编排器**。每个编排器接收你的 adapter（[实现 Adapter](/zh/how-to/implement-adapter) 中的 port 实现）与线上请求，运行协议门禁，经你的 ports 加载与持久化数据，并返回 `SpokeResult` —— 预期的拒绝从不抛异常。每个编排器都是异步入口：调用时用 `await`（TypeScript），或在 `async fn` 内 `.await`（Rust）。同样的调用也可以原样运行在[通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter)或[多对等节点路由器](/zh/how-to/multi-peer-routing)之上 —— 两者都可直接作为 `BaselinePorts` 实现接入。

## 编排器一览

| 编排器 | 请求 | 响应 | 执行内容 |
|--------|------|------|----------|
| `orchestrateUpsert(ports, request)` | `UpsertRequest` | `UpsertResponse` | 校验 → 状态门禁 → 批次唯一性 → OCC put |
| `orchestratePromote(ports, request)` | `PromoteRequest` | `PromoteResponse` | promote 验收门禁 → 合并目标 → OCC put |
| `orchestrateRelate(ports, request)` | `RelateRequest` | `RelateResponse` | 校验 → 创建/更新 OCC put |
| `orchestrateCheck(ports, request, runChecker)` | `CheckRequest` | `CheckResponse` | 解析规则 → 加载作用域 → 运行检查器 → 持久化 findings |
| `orchestrateAssemble(ports, request)` | `AssembleRequest` | `AssembleResponse` | 加载作用域 → 过滤 → 构建 `AssemblePacket` |
| `orchestrateProject(ports, request)` —— `l2-computable` | `ProjectRequest` | `ProjectResponse` | 校验 → `ComputablePort.project` |
| `orchestrateCompute(ports, request)` —— `l2-computable` | `ComputeRequest` | `ComputeResponse` | 校验 → `ComputablePort.compute` |
| `orchestrateForkCheck` / `orchestrateForkAssemble` —— `l5-fork` | fork 作用域请求 | 同形状响应 | 要求 `scope.fork_id` → fork 时间轴读取 |

## Upsert —— 创建或更新条目

```ts
import { orchestrateUpsert } from "@42ch/spoke-operations";
import type { UpsertRequest } from "@42ch/spoke-schemas";

async function runUpsert() {
  const result = await orchestrateUpsert(adapter, {
    knowledge_entries: [mira, harbor],
  });

  if (result.ok) {
    console.log(result.value.knowledge_entries.map((e) => e.entry_id));
  }
}
```

`UpsertRequest` 携带 1..n 条条目，外加可选的 `idempotency_key`（不透明提示 —— 线上语义由产品侧决定）。编排器逐条校验（`MISSING_REQUIRED_FIELD`、`EMPTY_CANONICAL_NAME` 等）、在条目已存在时做状态迁移门禁、检查批次内 active 唯一性，并以正确的期望基准修订号持久化。

## Promote —— 提取为持久条目

```ts
import { orchestratePromote } from "@42ch/spoke-operations";
import type { PromoteRequest } from "@42ch/spoke-schemas";

async function runPromote() {
  const result = await orchestratePromote(adapter, {
    candidate: provisionalEntry,      // 通常 status 为 "provisional"
    target_entry_id: "kb_existing",   // 可选合并目标
  });
}
```

Promote 运行验收门禁（`CANDIDATE_NOT_PROVISIONAL`、`CANDIDATE_TERMINAL_STATUS` 等）与修订门禁，应用验收状态迁移，并经由 `putKnowledgeEntry` 持久化。携带 `target_entry_id` 时，响应会带上被合并条目的 `superseded_id`。

## Relate —— 类型化有向边

```ts
import { orchestrateRelate } from "@42ch/spoke-operations";
import type { RelateRequest } from "@42ch/spoke-schemas";

async function runRelate() {
  const result = await orchestrateRelate(adapter, {
    relation: {
      schema_version: 1,
      relation_id: "rel_mira_harbor",
      relation_type: "located_in",
      from_id: "kb_mira",
      to_id: "kb_harbor",
      extensions: {},
    },
  });
}
```

Relation 校验区分创建与更新（`RELATION_SELF_EDGE`、`RELATION_MISSING_ENDPOINT` 等），OCC 感知的 put 在 adapter 内处理修订号分配。

## Check —— 在一个作用域上运行检查器

`orchestrateCheck` 先加载作用域规则与数据，再把 `CheckRunInput` 交给你 —— 你的检查器回调返回 `Finding[]`，编排器负责持久化：

```ts
import { orchestrateCheck, spokeOk, type CheckRunInput } from "@42ch/spoke-operations";
import type { CheckRequest } from "@42ch/spoke-schemas";

async function runCheck() {
  const result = await orchestrateCheck(adapter, checkRequest, (input: CheckRunInput) => {
    // input: { request, entries, events, rules }
    const findings = myChecker(input.entries, input.rules);
    return spokeOk(findings); // 或 spokeReject(SpokeRejectCode.INVALID_INPUT, "...")
  });
}
```

规则经 `RuleQueryPort` 从 `rule_refs` 解析，请求内嵌的 `rules[]` 按 `rule_id` 覆盖。`check` 只返回 findings —— 上下文包请用 `assemble`。

## Assemble —— 构建上下文包

```ts
import { orchestrateAssemble } from "@42ch/spoke-operations";
import type { AssembleRequest } from "@42ch/spoke-schemas";

async function runAssemble() {
  const result = await orchestrateAssemble(adapter, {
    scope: { scope_id: "book-harbor", entry_types: ["character"] },
    max_entries: 20, // 可选的条目数量提示
  });
}
```

编排器加载作用域、应用作用域过滤，并构建仅线上（wire-only）的 `AssemblePacket`，带保序截断。组装本身 —— ranking、retrieval、token 预算 —— 由产品侧完成。

## 处理拒绝

每个编排器都返回 `SpokeResult`：

```ts
import { SpokeRejectCode } from "@42ch/spoke-operations";

if (!result.ok) {
  switch (result.code) {
    case SpokeRejectCode.REVISION_CONFLICT:
    case SpokeRejectCode.STORED_REVISION_STALE:
      // 重新加载并使用新修订号重试
      break;
    case SpokeRejectCode.CAPABILITY_PORT_MISSING:
      // adapter 未实现该 op 所需的可选 port
      break;
    default:
      // 校验 / 状态拒绝 —— 上抛给调用方
  }
}
```

线上响应遵循与请求/响应信封相同的单一失败方言：响应要么是成功载荷，要么是 `{ "error": ErrorEnvelope }` —— 两者永不共存。库的拒绝可映射为你传输层携带的 `ErrorEnvelope` 形状。

## 纯度边界

编排器运行协议门禁并驱动你的 ports —— 它们不直接触碰存储、LLM 调用、ranking、retrieval 或传输。所有这些都由你的产品经注入的 adapter 提供。Finding 与 promote 生命周期（状态迁移、验收门禁）是库内的纯、持久化前规则；持久化经由你的 ports 完成。

## 下一步

- [操作线上参考（Ops wire）](/zh/reference/ops) —— 请求/响应信封字段表与 `Scope`。
- [实现 Adapter](/zh/how-to/implement-adapter) —— 每个编排器背后的 port 契约。
- [通读 ToyWorld 参考适配器](/zh/how-to/walk-toy-world) —— 已提交 fixture 图中的编排器用法。
