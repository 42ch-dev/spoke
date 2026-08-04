---
title: 通读 ToyWorld 参考适配器
---

# 通读 ToyWorld 参考适配器（Walk the ToyWorld reference adapter）

[`fixtures/toy-world/`](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) 是协议拥有的实操样例：一份已提交的 JSON 图（「Mira at Harbor」）、TypeScript 与 Rust 两套参考 `ToyWorldAdapter` 实现，以及一个把每个 fixture 对照已提交 schema 做校验的一致性测试台。它是你自己的 adapter 的起点骨架 —— 要复制的模式，而不是产品本身。

## 图背后的故事

制图师 Mira 于黎明抵达 Harbor Town；一条一致性规则标记出一条 open finding；一个 `AssemblePacket` 为该场景划定上下文。该图刻意覆盖线上面的各个角落：

- 一个**双重关注（dual-concern）对**把本体 `entry_type: "event"` 的 KnowledgeEntry `kb_tw_harbor_dawn_event` 与 TimelineEvent `evt_tw_harbor_dawn` 关联起来。
- 三个 moment 尺度节拍（beat）按顺序延伸 Harbor 主线 —— 集市询问、海关闸口检查（画像 `entry_type: "beat"`）、泊位确认 —— 由 `precedes` Relations 与每个 TimelineEvent 上的 `extensions.spoke.timeline_entry_id` 串联。
- Harbor Town 携带可选 `l2-computable` 的 `body.state` / `body.computable`（潮汐与货物）；黎明时刻记录 `computable_logs`。
- 两个 `HostCapabilityManifest` 主机（`host_tw_primary` / `host_tw_peer`）声明两两不相交的 `namespaces[]`，用于进程内协作。
- 六个 `conn_tw_*` fixture 在可选 `spoke-connect` 能力下演示一次两主机 connect 交换：双向签名握手、一份会话快照、一个包装真实 `op_tw_check_request` 载荷的 invoke 请求，以及成功与错误两条 invoke 响应分支。

## adapter 实现了什么

`ToyWorldAdapter` 实现 **`FullAdapter`** —— 基线 ports 加 `l2-computable` 与 `l5-fork`：

| Port 族 | ToyWorldAdapter 行为 |
|---------|----------------------|
| 基线 OCC 族（五个 port 族） | 可运行的内存 OCC；可选从已提交 fixture JSON 播种 |
| `HostManifestPort` | 自身清单来自 `host_tw_primary.json`；对端列表来自内存中的 `host_tw_peer.json`（排除自身、按 `host_id` 去重、升序排序） |
| `ComputablePort` | 由已提交 op fixture 合成的线上有效 `ProjectResponse` / `ComputeResponse`（回显请求 `session_id` / `entry_id`） |
| `ForkTimelineQueryPort` | 按 `scope.fork_id` 过滤的已播种时间轴事件 |

## TypeScript 侧

打开 `src/adapter/`：

- `memory-store.ts` —— `MemoryStore`：adapter 委托的内存 OCC 存储（带修订号检查的 get/put）。这是你自己的存储桥接要复制的模式。
- `toy-world-adapter.ts` —— `ToyWorldAdapter` 与 `asBaselineOnly()`：一个类型实现全部 ports；基线投影省略可选方法，使动态编排器返回 `CAPABILITY_PORT_MISSING`。
- `index.ts` —— 导出两者的桶文件（barrel）。

```ts
import { ToyWorldAdapter, asBaselineOnly } from "@42ch/spoke-fixture-toy-world";
import { orchestrateUpsert, orchestrateCheck } from "@42ch/spoke-operations";

const adapter = ToyWorldAdapter.withCommittedFixtures(); // 播种 kb/rel/evt/rule/fnd fixtures
// 或：new ToyWorldAdapter() 使用空存储

const baseline = asBaselineOnly(adapter); // 基线动态边界
```

一致性测试台（`tests/`）驱动同一份 adapter 源码：

- `toy-world-conformance.test.ts` —— 用 AJV 对照 schema 校验每个已提交 fixture。
- `toy-world-adapter.test.ts` —— 在已播种图上练习 port 编排（Vitest）。
- `toy-world-ops-exercise.test.ts` —— 对 adapter 走通各 op 族。

## Rust 侧

`rust/` 下的 crate `spoke-fixture-toy-world` 镜像 TypeScript adapter（`src/toy_world_adapter.rs`、`src/memory_store.rs`、`src/lib.rs`）：

```rust
use spoke_fixture_toy_world::{as_baseline_only, ToyWorldAdapter};

let adapter = ToyWorldAdapter::with_committed_fixtures();
// 或：ToyWorldAdapter::default() 使用空存储
let baseline = as_baseline_only(adapter);
```

`rust/tests/toy_world_adapter.rs` 在 cargo 中运行相同的 port 编排演示。该 crate `publish = false` —— 与 TypeScript fixture 软件包一样，是仅限工作区的参考。

## connect fixtures

`conn_tw_*` JSON 展示 toy-world 主机之间的完整两主机 connect 交换：双向 `ConnectHello`（`peer_tw_primary` ↔ `peer_tw_peer`）、一份 `ConnectSession` 快照（`initial_sequence: 0`）、一个包装真实 check 载荷的 `ConnectInvokeRequest`，以及两条 invoke 响应分支（成功分支内嵌 `fnd_tw_open`；错误分支复用共享 `error-envelope`，`INVALID_INPUT`）。握手 `signature` 字段是结构性测试向量 —— JCS 规范化与密码学校验属于参考栈。

## 复制骨架

1. 克隆 adapter 形状：一个类型实现你声明的 port 族（从 `BaselineAdapter` 起步，成长到 `FullAdapter`）。
2. 用你的存储替换 `MemoryStore`（同一方法面：带期望基准修订号检查的 get/put）。
3. 调用匹配的 `orchestrate*` —— 见[编排操作（orchestrate ops）](/zh/how-to/orchestrate-ops)。
4. 用相同的 AJV/Vitest 测试台模式对照已提交 schema 校验你的 fixtures（仓库内 `pnpm run test:fixtures`；Rust 侧 `cargo test -p spoke-fixture-toy-world`）。

## 下一步

- [实现 Adapter](/zh/how-to/implement-adapter) —— `ToyWorldAdapter` 背后的 port 契约。
- [编排操作（orchestrate ops）](/zh/how-to/orchestrate-ops) —— 测试台练习的 `orchestrate*` 调用。
- [数据模型参考](/zh/reference/data-model) —— fixtures 所校验的线上对象。
