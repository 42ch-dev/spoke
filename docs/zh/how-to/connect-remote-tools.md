---
title: 暴露并调用远程工具
---

# 暴露并调用远程工具（Expose and invoke remote tools）

一条 connect 会话承载两个方向的能力流量：主机向拨号方提供其 `BaselinePorts` 面供其消费（`port.*` 方向）；拨号方则可以提供**工具（tools）**，主机从已认证清单中发现这些工具，并在编排中途反向调用它们。本指南端到端覆盖工具方向 —— 通告、注册、发现、调用、回填 —— 代码片段来自可运行的 demo（`examples/connect-demo/`，TypeScript）与参考提供方（`fixtures/toy-world/`，TypeScript + Rust），并与其逐字节一致。

一句话概括整个故事：客户端的清单在 `tools[]` 中通告工具；主机从已认证清单中列出这些工具，在编排步骤中反向调用其中一个，并把结果回填进一次 `BaselinePorts` 调用。demo 的工具是 `tools.toy_world.roll_dice`（确定性骰子）与 `tools.toy_world.lore_lookup`（只读 lore 查询）—— 在 demo 与参考提供方中 id 冻结一致。

## 1. 在清单中通告工具

清单中用三个字段描述拨号方通告、供主机在已建立会话上发现并反向调用的工具：

| 字段 | 作用 |
|------|------|
| `capabilities[]` | 列出工具能力字符串（如 `tools.toy_world.roll_dice`），使双方像协商其它能力一样协商这些工具 |
| `namespaces[]` | 声明该清单拥有的 namespace；每个已声明工具的 namespace 必须列入 |
| `tools[]` | 为每个工具携带完整的 `ToolDescriptor` —— 能力 id、线上 op、描述、参数/结果子 schema 与幂等性 |

demo 客户端的清单展示了该形状：

```ts
export const DEMO_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: [
    "spoke-baseline",
    TOY_WORLD_ROLL_DICE_ID,
    TOY_WORLD_LORE_LOOKUP_ID,
  ],
  namespaces: [DEMO_SCOPE_ID, "toy_world"],
  tools: [ROLL_DICE_DESCRIPTOR, LORE_LOOKUP_DESCRIPTOR],
  extensions: {},
};
```

工具 id 遵循语法 `tools.<ns>.<tool_id>`（`^tools\.[a-z][a-z0-9_-]*\.[a-z0-9][a-z0-9_-]*$`）：`tools.toy_world.roll_dice` 的 namespace 是 `toy_world`、工具 id 是 `roll_dice`。工具的线上 `op` 等于其 `capability_id` —— 能力字符串就是 op 字符串。

`validateManifestTools`（来自 `@42ch/spoke-operations`）在清单被用于发现之前对其检查：每个描述符格式正确、其 `capability_id` 出现在 `capabilities[]` 中、其 namespace 被 `namespaces[]` 拥有、且工具 id 唯一。主机在发现时对拨号方的清单运行同样的检查。完整描述符字段见[清单 `tools[]` 字段表](/zh/reference/connect#清单-tools-字段表)。

## 2. 在 RemoteAdapter 上注册处理器

`RemoteAdapter`（远程适配器）通过已注册的处理器服务反向调用。注册发生在已建立的 adapter 上，紧随拨号之后：

```ts
adapter.registerToolHandler(TOY_WORLD_ROLL_DICE_ID, rollDice);
adapter.registerToolHandler(
  TOY_WORLD_LORE_LOOKUP_ID,
  loreLookup(loreStore),
);
```

处理器接收工具参数对象，并以 `SpokeResult` 结算：

```ts
type ToolHandler = (
  args: Record<string, unknown>,
) => Promise<SpokeResult<unknown>>;
```

demo 的 `rollDice` 处理器是确定性的 —— 种子由参数推导，因此相同参数总是产生相同掷骰结果：

```ts
export function rollDice(args: Record<string, unknown>): Promise<ToolResult> {
  const count = args["count"];
  const sides = args["sides"] ?? 6;
  if (!isPositiveInteger(count)) {
    return Promise.resolve(
      reject("INVALID_INPUT", "roll_dice count must be a positive integer", {
        field: "count",
      }),
    );
  }
  if (!isPositiveInteger(sides) || sides < 2) {
    return Promise.resolve(
      reject("INVALID_INPUT", "roll_dice sides must be an integer >= 2", {
        field: "sides",
      }),
    );
  }

  const random = mulberry32(fnv1a(`${count}:${sides}`));
  const rolls: number[] = [];
  for (let index = 0; index < count; index += 1) {
    rolls.push(1 + Math.floor(random() * sides));
  }
  const total = rolls.reduce((sum, roll) => sum + roll, 0);
  return Promise.resolve({ ok: true, value: { rolls, total } });
}
```

工具的 `ToolDescriptor` 描述该 ABI —— 参数子 schema、结果子 schema 与咨询性元数据：

```ts
export const ROLL_DICE_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: TOY_WORLD_ROLL_DICE_ID,
  op: TOY_WORLD_ROLL_DICE_ID,
  description:
    "Roll `count` dice with `sides` faces each. Deterministic: the same arguments always produce the same rolls (seeded from the arguments).",
  input: {
    type: "object",
    properties: {
      count: { type: "integer", minimum: 1 },
      sides: { type: "integer", minimum: 2 },
    },
    required: ["count"],
  },
  output: {
    type: "object",
    properties: {
      rolls: {
        type: "array",
        items: { type: "integer" },
      },
      total: { type: "integer" },
    },
    required: ["rolls", "total"],
  },
  idempotent: true,
};
```

注册从不修改清单 —— 用于发现的描述符真源保持在 `tools[]`（经握手发送）。注册是 `validateManifestTools` 无法看到的运行时状态：它只检查清单内部一致性（描述符格式、能力成员、namespace 归属、id 唯一性），从不检查处理器注册表。为清单未声明的工具注册处理器是提供方缺陷，会在调用时暴露 —— 该工具永不可发现或协商，因此反向调用被拒绝（`op_unsupported` → `CAPABILITY_PORT_MISSING`），绝不静默成功。为非 `tools.` id 注册处理器是语法错误，会抛出异常。对同一 id 的重复注册覆盖前一个处理器（后者胜出）。

Rust 提供方以 `register_tool_handler` 镜像同一面，其处理器类型为 `Arc<dyn Fn(Value) -> BoxFuture<'static, SpokeResult<Value>> + Send + Sync>`：

```rust
use std::sync::Arc;
use serde_json::Value;

adapter.register_tool_handler(
    TOY_WORLD_ROLL_DICE_ID,
    Arc::new(|args: Value| Box::pin(async move { roll_dice(&args) })),
);
```

参考提供方的 [`default_tool_handlers`](https://github.com/42ch-dev/spoke/blob/main/fixtures/toy-world/rust/src/toy_world_tools.rs) 以这一模式构建两个处理器 —— `roll_dice` 与绑定到 adapter 存储的 `lore_lookup`。

## 3. 从已认证清单中发现工具

主机从已认证清单得知拨号方能提供哪些工具 —— 即经验证的握手所内嵌的 `host`，在响应方侧缓存为 `remoteManifest`：

```ts
const manifest: HostCapabilityManifest = responder.remoteManifest;
const validated = validateManifestTools(manifest);
const discovered = listTools(manifest).map(
  (descriptor) => descriptor.capability_id,
);
```

`validateManifestTools` 返回 `SpokeResult`；`listTools` 按声明顺序返回描述符。对 demo 客户端，这列出两个工具：

```text
tools.toy_world.roll_dice
tools.toy_world.lore_lookup
```

发现是已认证会话的属性，而非独立的通告步骤 —— 主机读取的是它在握手时已校验的清单。

## 4. 在编排中途反向调用工具

主机用 `invokeTool` 反向调用已发现的工具 —— 这是反方向的一条普通签名 connect invoke，携带工具参数：

```ts
const result = await responder.invokeTool(
  TOY_WORLD_ROLL_DICE_ID,
  { ...ORCHESTRATION_ROLL_ARGS }, // { count: 2, sides: 6 }
);
```

demo 主机在其 `putKnowledgeEntry` 编排内运行此步骤，紧随客户端提交 compass 之后。成功时 `result` 是处理器返回的值 —— 对 2d6，种子算法总是产生 `{ rolls: [1, 2], total: 3 }`。主机随后把该值作为 `demo-harbor/artifact/dice-roll` 回填进引擎 —— 一次客户端在下次 `listKnowledgeEntries` 中可见的 `BaselinePorts` 步骤。

反向调用与其它 op 使用同一拒绝词汇：调用拨号方未列出（因此未协商）的工具，会以线上码 `op_unsupported` 应答，库将其映射为带 `details.wire_code = "op_unsupported"` 的 `CAPABILITY_PORT_MISSING` 拒绝。demo 主机记录该拒绝且不回填任何内容 —— 编排呈现拒绝而非静默成功。客户端的清单必须列出该工具，且双方必须协商它：`tools.*` op 仅当能力字符串本身在会话的 `negotiated_capabilities`（协商能力）中时才被分派。见线上参考中的[反向调用语义](/zh/reference/connect#反向调用语义)。

demo 服务器以其自身清单、allowlist 与作为 `ports` 的编排器接线响应方：

```ts
void connectResponder({
  transport,
  identity: { seed: DEMO_SERVER_SEED },
  manifest: DEMO_SERVER_MANIFEST,
  allowlist: [DEMO_CLIENT_PEER_ID],
  peerKeys: {
    [DEMO_CLIENT_PEER_ID]: DEMO_CLIENT_PUBKEY,
  },
  ports: orchestrator,
}).then((responder) => {
  orchestrator.setResponder(responder);
});
```

`connectResponder` 接受与 adapter 相同的每次调用超时旋钮：`invokeTimeoutMs`（可选；默认 5000）。超时仅使该次反向调用失败 —— 会话保持可用。

服务器的清单声明与客户端所服务的相同的工具 id，因此两个方向在同一会话上协商。

## 5. 运行 demo

demo 在两个终端中通过真实 WebSocket 运行（构建集见[demo README](https://github.com/42ch-dev/spoke/blob/main/examples/connect-demo/README.md)）：

```bash
node examples/connect-demo/server/dist/main.js --port 8787
```

```bash
node examples/connect-demo/client/dist/main.js --url ws://127.0.0.1:8787
```

客户端打印每个故事步骤 —— 拨号、已注册工具、主机反向调用 `roll_dice` 期间发生的 put、包含 `demo-harbor/artifact/dice-roll` 的条目列表，以及 findings 往返。e2e 门禁在临时端口上启动主机，并断言整条路径，包括确定性掷骰值与能力拒绝路径：

```bash
pnpm -F @42ch/spoke-demo-client test
```

## 下一步

- [connect 线上参考](/zh/reference/connect) —— 清单 `tools[]` 字段表、`tools.*` 分派规则与反向调用语义。
- [Connect 架构](/zh/explanation/connect) —— 工具与 ports 背后的双向能力流。
- [ToyWorld 参考适配器走读](/zh/how-to/walk-toy-world) —— TypeScript 与 Rust 的可复制提供方。
- [通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter) —— 本指南所基于的拨号与 `port.*` 方向。
- [从原生绑定使用 RemoteAdapter](/zh/how-to/remote-adapter-native-binding) —— 同一工具契约的 FFI 面（C#、Go、Kotlin、Python、Swift）。
