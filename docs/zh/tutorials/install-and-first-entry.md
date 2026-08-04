---
title: 安装并创建你的第一条 KnowledgeEntry
---

# 安装并创建你的第一条 KnowledgeEntry（Install and create your first KnowledgeEntry）

本教程带你完成第一次完整的往返（round-trip）：安装线上类型（wire types）与操作库软件包，构建一个最小的内存 adapter，通过 `orchestrateUpsert` upsert 一条 KnowledgeEntry（知识条目），并把持久化后的条目读回来。全程基于**已发布软件包**（npm 与 crates.io），无需检出本仓库。

配套教程[开启你的首个 connect 会话](/zh/tutorials/first-connect-session)会在此基础上加入身份推导、allowlist、签名握手与会话关联，走通 WebSocket 上的完整链路。

## 开始之前

- **TypeScript 路径** —— Node.js ≥ 20.19，带 `pnpm`。
- **Rust 路径** —— 稳定的 Rust 工具链，带 `cargo`。

所有 SPOKE 软件包共用一套锁步 SemVer（`X.Y.Z`）。你安装的每个软件包都应固定到同一个版本。

## 1. 安装软件包

TypeScript（npm）：

```bash
pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z
```

Rust（crates.io）：

```bash
cargo add spoke-schemas@X.Y.Z spoke-operations@X.Y.Z
```

你会得到：

- **`@42ch/spoke-schemas` / `spoke-schemas`** —— 由 [`schemas/`](https://github.com/42ch-dev/spoke/tree/main/schemas) 下的 JSON Schema 单一事实来源（SSOT）生成的线上类型（`KnowledgeEntry`、`UpsertRequest`、`UpsertResponse` 等）。
- **`@42ch/spoke-operations` / `spoke-operations`** —— 纯函数生命周期辅助、按能力切片的 adapter ports，以及在持久化之前执行协议门禁的 `orchestrate*` 入口。

## 2. 创建一条 KnowledgeEntry

KnowledgeEntry 是知识库的原子单元：稳定的 `entry_id`、开放的 `entry_type` / `status` 字符串、封闭的 `body`，以及必填的 `extensions` 字段袋。

TypeScript：

```ts
import type { KnowledgeEntry } from "@42ch/spoke-schemas";

const mira: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "kb_mira",
  entry_type: "character",
  canonical_name: "Mira Vale",
  status: "provisional",
  body: { summary: "Reluctant cartographer arriving at Harbor Town." },
  extensions: { my_product: { world_id: "wld_harbor" } },
};
```

Rust —— 生成的类型提供类型化 builder，构造过程由编译器校验：

```rust
use spoke_schemas::data::KnowledgeEntry;

let mira = KnowledgeEntry::builder()
    .schema_version(1)
    .entry_id("kb_mira")
    .entry_type("character")
    .canonical_name("Mira Vale")
    .status("provisional")
    .body(
        spoke_schemas::data::KnowledgeEntryBody::builder()
            .summary("Reluctant cartographer arriving at Harbor Town.".to_string())
            .build()?,
    )
    .extensions(std::collections::HashMap::new())
    .build()?;
```

## 3. 实现一个最小 adapter

`orchestrateUpsert` 先执行校验与状态门禁，再经由你的 ports 完成持久化。adapter 是协议与你的存储之间的桥梁 —— 本教程用一个内存 `Map` 实现基线 port 族。

TypeScript：

```ts
import { SpokeRejectCode, spokeOk, spokeReject, type BaselinePorts, type SpokeResult } from "@42ch/spoke-operations";
import type { Finding, HostCapabilityManifest, KnowledgeEntry, Relation, Rule, Scope, TimelineEvent } from "@42ch/spoke-schemas";

class InMemoryAdapter implements BaselinePorts {
  private entries = new Map<string, KnowledgeEntry>();

  async getKnowledgeEntry(entryId: string): Promise<SpokeResult<KnowledgeEntry>> {
    const entry = this.entries.get(entryId);
    return entry === undefined
      ? spokeReject(SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND, `no entry ${entryId}`)
      : spokeOk(entry);
  }

  async putKnowledgeEntry(entry: KnowledgeEntry, expectedBaseRevision: number | null): Promise<SpokeResult<KnowledgeEntry>> {
    const stored = this.entries.get(entry.entry_id);
    if ((stored?.revision ?? null) !== expectedBaseRevision) {
      return spokeReject(SpokeRejectCode.REVISION_CONFLICT, "revision mismatch");
    }
    this.entries.set(entry.entry_id, entry);
    return spokeOk(entry);
  }

  // Relation / Scope / Finding / Rule / Host-manifest 各 port —— 本教程返回
  // 空默认值；生产 adapter 接入真实存储。
  async getRelation(): Promise<SpokeResult<Relation>> { return spokeReject(SpokeRejectCode.RELATION_NOT_FOUND, "no relations"); }
  async putRelation(): Promise<SpokeResult<Relation>> { return spokeReject(SpokeRejectCode.INVALID_INPUT, "no relations"); }
  async listKnowledgeEntries(_scope: Scope): Promise<SpokeResult<KnowledgeEntry[]>> { return spokeOk([...this.entries.values()]); }
  async listTimelineEvents(_scope: Scope): Promise<SpokeResult<TimelineEvent[]>> { return spokeOk([]); }
  async putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>> { return spokeOk(findings); }
  async listRules(): Promise<SpokeResult<Rule[]>> { return spokeOk([]); }
  async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
    return spokeOk({ schema_version: 1, host_id: "host_tutorial", roles: ["data-store"], capabilities: ["spoke-baseline"], namespaces: ["tutorial"], extensions: {} });
  }
  async listPeerHostCapabilityManifests(): Promise<SpokeResult<HostCapabilityManifest[]>> { return spokeOk([]); }
}
```

Rust —— fixture crate 在参考 `ToyWorldAdapter` 上实现了完全相同的 port traits；完整的 Rust port 实现见[通读 ToyWorld 参考适配器](/zh/how-to/walk-toy-world)。`spoke-operations` 会 re-export 线上类型（`spoke_operations::spoke_schemas`），一个依赖即可同时获得两者。

## 4. 执行 upsert 往返

TypeScript：

```ts
import { orchestrateUpsert } from "@42ch/spoke-operations";
import type { UpsertRequest } from "@42ch/spoke-schemas";

const adapter = new InMemoryAdapter();
const request: UpsertRequest = { knowledge_entries: [mira] };

async function runUpsert() {
  const result = await orchestrateUpsert(adapter, request);

  if (result.ok) {
    const persisted = result.value.knowledge_entries[0];
    console.log(persisted.entry_id, persisted.status); // kb_mira provisional
  } else {
    console.error(result.code, result.message);
  }
}
```

预期的拒绝以 `SpokeResult` 返回 —— 一个可判别的 `{ ok: true }` / `{ ok: false, code, message }` 联合 —— 而不是抛异常，因此你可以按 `SpokeRejectCode`（`CANDIDATE_NOT_PROVISIONAL`、`REVISION_CONFLICT` 等）分支处理。

Rust：

```rust
use spoke_operations::{orchestrate_upsert, BaselinePorts};
use spoke_operations::spoke_schemas::{UpsertRequest, UpsertResponse};

async fn upsert(adapter: &impl BaselinePorts, request: UpsertRequest) {
    match orchestrate_upsert(adapter, request).await {
        Ok(response) => println!("persisted: {:?}", response.knowledge_entries),
        Err(reject) => println!("rejected: {} — {}", reject.code, reject.message),
    }
}
```

## 5. 编排器替你做了什么

`orchestrateUpsert` 对每条条目依次执行：schema 校验、状态迁移门禁（条目已存在时）、批次内的 active 唯一性检查，以及携带期望基准修订号的乐观并发 `putKnowledgeEntry`。你的 adapter 保持纯 I/O —— 所有协议规则都在库内完成。

## 下一步

- [实现 Adapter](/zh/how-to/implement-adapter) —— 你声明的能力需要实现哪些 port 族。
- [编排操作（orchestrate ops）](/zh/how-to/orchestrate-ops) —— 通过同一 port 面执行 promote、relate、check 与 assemble。
- [开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 与另一个 SPOKE 主机对话。
- [通读 ToyWorld 参考适配器](/zh/how-to/walk-toy-world) —— 已提交的「Mira at Harbor」样例图，以及 TypeScript 与 Rust 两套完整 `FullAdapter` 实现。
