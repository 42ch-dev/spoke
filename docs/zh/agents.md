---
title: 面向 Agent
---

# 面向 Agent

本页是代码助手与自动化集成方的阅读契约。站点其余页面面向人类集成者；本页说明 Agent 应读哪里、哪个来源优先，以及生成代码时最常见的错误。

## 事实优先级

来源冲突时，按此顺序取信：

| 优先级 | 来源 | 理由 |
|--------|------|------|
| 1 | `schemas/` | 手工编写的 JSON Schema。字段名、必填集合、模式与封闭对象在此定义 |
| 2 | `.mstar/specs/` | 规范性协议散文：分层、数据模型、ops、memory、connect、绑定 |
| 3 | `/zh/reference/*` | 由 schema 派生的字段表与信封 |
| 4 | `/zh/how-to/*`、`/zh/tutorials/*` | 操作步骤与代码样例；它们以 reference 事实为前提 |
| 5 | `/zh/explanation/*` | 概念与边界 |
| 6 | `README.md` | 人类入口；不是规范来源 |

文档与 `schemas/` 冲突时，按文档缺陷处理。不要依据矛盾处生成代码。

## 阅读顺序

1. `/zh/reference/protocol` —— schema 清单、能力标志、`extensions` 与 `modules` 契约。
2. `/zh/reference/data-model` —— 对象字段。写任何字面量前先读该对象。
3. `/zh/reference/ops` —— 请求与响应信封、`Scope`、`ErrorEnvelope`。
4. 需要的能力参考：`/zh/reference/memory`、`/zh/reference/mind-state`、`/zh/reference/connect`。
5. `/zh/how-to/implement-adapter` 与 `/zh/how-to/orchestrate-ops` —— port 与 `orchestrate*` 序列。
6. `/zh/explanation/concepts` —— 名称有歧义时，查分层与双关注对。

## 任务路由

| 任务 | 先读 | 再读 |
|------|------|------|
| 持久化 KnowledgeEntry 数据 | `/zh/reference/data-model` | `/zh/how-to/implement-adapter` |
| 调用 op | `/zh/reference/ops` | `/zh/how-to/orchestrate-ops` |
| 新增能力标志 | `/zh/reference/protocol` | `/zh/explanation/concepts` |
| 提供 memory 服务 | `/zh/reference/memory` | `/zh/how-to/implement-adapter` |
| 建立跨进程会话 | `/zh/reference/connect` | `/zh/how-to/connect-ts-client` |
| 通过传输代理 adapter port | `/zh/how-to/connect-remote-adapter` | `/zh/reference/connect` |
| 调用或暴露远程工具 | `/zh/reference/connect` | `/zh/how-to/connect-remote-tools` |
| 从其他语言绑定 | `/zh/how-to/connect-native-bindings` | `/zh/how-to/remote-adapter-native-binding` |
| 固定版本 | `/zh/packages/quick-start` | `/zh/release/versioning` |

## 能力标志

`spoke-baseline` 是唯一的基线声明。其余标志均为附加能力，需显式声明。

| 标志 | 增加的内容 |
|------|-----------|
| `l2-computable` | `body.state` / `body.computable`、`TimelineEvent.computable_logs`、`project` 与 `compute` |
| `l5-fork` | TimelineEvent 上的 `fork_id` / `parent_fork_id`、`Scope.fork_id` |
| `spoke-cognition` | `modules.cognition`、`Scope.cognition`、`modules.observation`、派生的 `MindState` |
| `narrative-modules` | KnowledgeEntry、AssemblePacket 与 TimelineEvent 上的 `modules` 字段袋 |
| `ke-extraction` | `extract` op 族与 `ExtractionPort` |
| `ke-ownership` | `KnowledgeEntry.owner` / `disclosure`、`Scope.viewpoint` |
| `spoke-connect` | 六个 connect 信封 |
| `spoke-memory` | 13 个 memory 模型与 33 个 `memory.*` ops；`spoke-memory-reflection` 与 `spoke-memory-transfer` 为子标志 |

## 常见陷阱

- `Rule` 是检查器**输入**；`Finding` 是检查器**输出**。`entry_type: "rule"` 是本标签，不是 L6 对象。
- `TimelineEvent` 是 L5 时间轴对象。`entry_type: "event"` 是本体标签；同一概念可映射到两者。
- `MindState` 是派生物。权威值保留在持有者 KnowledgeEntry 的 `modules.cognition` 与 `modules.mental`。
- Connect 信封在已提交 schema 中携带 `signature`，它描述 protocol_version 2 的线形状。
- 序列按方向独立，从 0 开始。响应回显 `session_id`、`sequence` 与 `request_id`。
- ops 结果是 `SpokeResult`。按 `ok` 与 `SpokeRejectCode` 分支，不要按抛出的异常类型分支。
- adapter 写入受乐观并发控制：`expectedBaseRevision: null` 表示创建；数字必须等于已存 revision。
- op 分发会在任何处理器运行前拒绝。未知或未协商的工具、静态 `port.*` 能力族、`extract`，以及携带 viewpoint 但未协商 `ke-ownership` 的 Scope 应答 `op_unsupported`。`capability_missing` 应答已登记但能力族未协商的 `memory.*` op，以及未协商 `spoke-cognition` 却携带认知负载的请求（`Scope.cognition` 或 `cognition_filter`）。不要模拟未声明的能力族。

## 机器可读包

- [`/llms.txt`](/llms.txt) —— 每页一行的索引。
- [`/llms-full.txt`](/llms-full.txt) —— 按阅读顺序拼接的全部英文页面，由 `tooling/docs/build-llms.mjs` 从页面源生成。
