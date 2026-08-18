---
title: 集成 RemoteAdapter 连接推理主机
---

# 集成 RemoteAdapter 连接推理主机（Integrate a RemoteAdapter against a live host）

本教程通过真实 WebSocket 把 **RemoteAdapter（远程适配器）** 连接到运行中的 SPOKE connect 主机：你运行 demo 模拟推理主机，实现 adapter 拨号所需的消息导向 `Transport`（传输接口），调用即插即用的 `BaselinePorts` 面，观察主机的推理引擎从你的数据推导出产物，并看到失败如何呈现。你操作的代码是 `examples/connect-demo/` 中随仓库发布的 demo —— 与第三方 TypeScript 应用会使用的集成面完全相同：**语言原生 TypeScript 客户端** `@42ch/spoke-connect`（其 `./remote` 子路径）、`@42ch/spoke-schemas` 线上类型，以及一个 WebSocket 库。仅此而已。

建议先完成[开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 本教程通过库直接使用身份、allowlist、签名握手与会话概念，这些概念在该教程中建立。

## 1. 认识 demo 主机

demo 由 `examples/connect-demo/` 下的两个软件包组成：

- `server/`（`@42ch/spoke-demo-server`）—— 一个确定性的**模拟推理主机（mock inference host）**：由纯规则引擎支撑的 `BaselinePorts` adapter，经 `ws` WebSocketServer 由符合规范的 connect 响应方提供服务。
- `client/`（`@42ch/spoke-demo-client`）—— **第三方视角**：它自己在 `ws` 之上实现 `Transport`，然后由真实库客户端（来自 `@42ch/spoke-connect/remote` 的 `connectRemoteAdapter`）拨号主机，并调用即插即用的异步 `BaselinePorts` 面。

主机的身份与能力来自它的 manifest。demo 服务器以 `demo-inference-host` 自报身份，携带 baseline 能力、两个 toy-world 工具能力 id（这样客户端的工具会在会话上被协商）与可选的 `l2-computable` / `l5-fork` 族；其命名空间为 `demo-harbor` 与 `toy_world`（`examples/connect-demo/server/src/adapter/mock-adapter.ts`）：

```ts
export const DEMO_SERVER_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-inference-host",
  roles: ["checker", "assembler"],
  capabilities: [
    "spoke-baseline",
    TOY_WORLD_ROLL_DICE_ID,
    TOY_WORLD_LORE_LOOKUP_ID,
    "l2-computable",
    "l5-fork",
  ],
  namespaces: [DEMO_SCOPE_ID, TOY_WORLD_NAMESPACE],
  tools: [ROLL_DICE_DESCRIPTOR, LORE_LOOKUP_DESCRIPTOR],
  extensions: {},
};
```

`TOY_WORLD_*` 与 `ROLL_DICE_*` / `LORE_LOOKUP_*` 常量是 `tools/toy-world-tools.ts` 中冻结的工具 id 与描述符 —— 工具方向见[暴露并调用远程工具](/zh/how-to/connect-remote-tools)；本教程跟随 port 方向。

`DEMO_SCOPE_ID` 是 demo 命名空间 `"demo-harbor"` —— 每个种子实体与每条 demo manifest 都归属其中。manifest 背后是 `MockEngine`，一个确定性的推理引擎：它从固定种子语料出发（两条 KnowledgeEntry（知识条目）—— 码头工人 Mira 与 Harbor 街区 —— 外加一条 relation、一条 rule，以及一条含三个事件的种子风暴分支时间线），接受带乐观并发（optimistic concurrency，OCC）的条件 put，并在每次被接受的变更之后重新推导自己的产物。推导是存储历史的纯函数：没有墙钟，没有随机性。引擎还拥有可选族的状态：`l2-computable` 会话（`project` 从请求的静态状态物化可计算视图；`compute` 合并增量并把结果结算回状态）与基于种子分支的 `l5-fork` 时间线查询。

主机对谁能拨号采取 fail-closed：它的 allowlist 恰好包含一个 `peer_id`（对等节点标识）—— demo 客户端的。客户端侧则只需要主机的公钥与主机的 `peer_id` 就能信任这条连接。

## 2. 运行主机

你需要 Node.js ≥ 20 与 `pnpm`。克隆仓库并安装一次：

```bash
git clone https://github.com/42ch-dev/spoke.git
cd spoke
pnpm install
```

CLI 从构建产物运行。构建已构建的 CLI 在运行时导入的工作区软件包，以及 demo 软件包本身（`examples/connect-demo/README.md`）：

```bash
pnpm -F @42ch/spoke-schemas build        # compile-time prerequisite: generated wire types
pnpm -F @42ch/spoke-connect build        # runtime dep of both built demo CLIs
pnpm -F @42ch/spoke-operations build     # runtime dep of the built server CLI
pnpm -F @42ch/spoke-demo-server build
pnpm -F @42ch/spoke-demo-client build
```

在**终端 1** 启动主机：

```bash
node examples/connect-demo/server/dist/main.js --port 8787
```

它打印自己的身份、allowlist 与监听 URL：

```text
SPOKE connect demo — mock inference host
  peer_id:   12D3KooWNm5t4HypYRmiC5v9CD2TnPKrJh2J8TcfJ2gPhA7L8TiZ
  allowlist: 12D3KooWM82bDYYgzgXaayHDdVciFe3bGvJ69qHnbSztNUJ933VQ
  listening: ws://127.0.0.1:8787
  tools:     discovers dialer tools from the authenticated manifest;
             reverse-invokes tools.toy_world.roll_dice mid-orchestration
  (Ctrl+C to stop)
```

`peer_id` 是主机的信任根 —— 由它的 Ed25519 公钥推导而来，与第一个教程完全一致。打印出的 allowlist 是 demo 客户端的 `peer_id`：这台主机只接受来自该对等节点的拨号。让主机保持运行。

## 3. 实现 `WsTransport`

`Transport` 是由消费方实现的接缝（seam）：在 adapter 与远端对等节点之间搬运 connect 信封。它是**消息导向**的 —— 一次调用恰好移动一个 connect 信封：`send(envelope)` 发送一个，`recv()` 返回下一个入站信封（连接关闭时拒绝），`close()` 释放资源且幂等。完整契约表见[通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter)。

demo 客户端在 `ws` WebSocket 软件包之上实现该接缝（`examples/connect-demo/client/src/transport/ws-transport.ts`）：

```ts
import { WebSocket } from "ws";

import type { EnvelopeBytes, Transport } from "@42ch/spoke-connect/remote";

/** A pending `recv` waiter. */
type RecvWaiter = {
  resolve: (bytes: EnvelopeBytes) => void;
  reject: (error: Error) => void;
};

/** View a `ws` message payload as envelope bytes (fresh per message). */
function toEnvelopeBytes(data: unknown): EnvelopeBytes {
  if (Buffer.isBuffer(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  return new Uint8Array(data as ArrayBuffer);
}
```

WebSocket 已经为消息分帧，因此一条 WS 消息恰好是一个 connect 信封 —— 这种载体上无需长度前缀定界。类保留一个入站缓冲区，容纳任何 `recv` 被调用之前到达的消息，以及另一方向上的挂起 `recv` 等待者队列：

```ts
export class WsTransport implements Transport {
  readonly #socket: WebSocket;
  /** Resolves once the socket is open; rejects if the connect fails. */
  readonly #open: Promise<void>;
  #closed = false;
  readonly #buffer: EnvelopeBytes[] = [];
  readonly #waiters: RecvWaiter[] = [];

  constructor(url: string) {
    this.#socket = new WebSocket(url);
    this.#open = new Promise<void>((resolve, reject) => {
      this.#socket.once("open", () => resolve());
      this.#socket.once("error", (error) => {
        reject(
          error instanceof Error
            ? error
            : new Error(`ws connect to ${url} failed`),
        );
      });
    });
    this.#socket.on("message", (data) => this.#push(toEnvelopeBytes(data)));
    // Both events fail pending recvs — a drop always surfaces as close/error.
    const fail = (): void => this.#failPending(new Error("ws connection closed"));
    this.#socket.on("close", fail);
    this.#socket.on("error", fail);
  }
```

`send` 等待 socket 打开，然后写入一个信封：

```ts
  async send(envelope: EnvelopeBytes): Promise<void> {
    await this.#open;
    if (this.#closed || this.#socket.readyState !== WebSocket.OPEN) {
      throw new Error("WsTransport is closed");
    }
    await new Promise<void>((resolve, reject) => {
      this.#socket.send(envelope, (error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve();
      });
    });
  }
```

`recv` 先服务缓冲消息，再进入等待；`close` 幂等，并让所有挂起的 `recv` 失败，使 adapter 进行中的 invoke 快速失败而不是等待超时：

```ts
  recv(): Promise<EnvelopeBytes> {
    if (this.#closed) {
      return Promise.reject(new Error("WsTransport is closed"));
    }
    const buffered = this.#buffer.shift();
    if (buffered !== undefined) {
      return Promise.resolve(buffered);
    }
    return new Promise<EnvelopeBytes>((resolve, reject) => {
      this.#waiters.push({ resolve, reject });
    });
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#failPending(new Error("WsTransport is closed"));
    this.#socket.close();
  }

  #push(bytes: EnvelopeBytes): void {
    const waiter = this.#waiters.shift();
    if (waiter !== undefined) {
      waiter.resolve(bytes);
      return;
    }
    this.#buffer.push(bytes);
  }

  #failPending(error: Error): void {
    for (const waiter of this.#waiters.splice(0)) {
      waiter.reject(error);
    }
  }
}
```

这就是整个接缝。adapter 发送或接收的每一条 connect 信封都流经这三个方法；adapter 在其上处理全部会话规则。

## 4. 用 `connectRemoteAdapter` 拨号

拿到 transport 之后，拨号就是一次调用。`connectRemoteAdapter` 执行签名握手交换、allowlist 检查与会话快照校验，然后解析为已建立的 adapter（`examples/connect-demo/client/src/main.ts`）：

```ts
export async function runDemoClient(options: {
  url: string;
}): Promise<DemoClientRun> {
  const transport = new WsTransport(options.url);
  const adapter = await connectRemoteAdapter({
    transport,
    localIdentity: { seed: DEMO_CLIENT_SEED },
    localManifest: DEMO_CLIENT_MANIFEST,
    remotePubkey: DEMO_SERVER_PUBKEY,
    allowlist: [DEMO_SERVER_PEER_ID],
  });
```

这些选项对应第一个教程的会话概念：

- `transport` —— 你的 `Transport` 实现；adapter 经它收发信封。
- `localIdentity.seed` —— 你的 32 字节 Ed25519 种子；adapter 用它签署你的握手。
- `localManifest` —— 你的 `HostCapabilityManifest`（主机能力清单），在签名握手中通告。demo 客户端是 `demo-harbor` 与 `toy_world` 命名空间中的 `input-source` 应用；其清单声明 baseline 能力、它服务的两个工具 id，以及可选的 `l2-computable` / `l5-fork` 族（一个族只有在双方 manifest 都声明时才会被协商）：

```ts
export const DEMO_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: [
    "spoke-baseline",
    TOY_WORLD_ROLL_DICE_ID,
    TOY_WORLD_LORE_LOOKUP_ID,
    "l2-computable",
    "l5-fork",
  ],
  namespaces: [DEMO_SCOPE_ID, "toy_world"],
  tools: [ROLL_DICE_DESCRIPTOR, LORE_LOOKUP_DESCRIPTOR],
  extensions: {},
};
```

- `remotePubkey` —— 主机的 32 字节 Ed25519 公钥。远端 `peer_id` 由它推导，且必须在 allowlist 上（fail-closed）。demo 携带固定身份种子，这些种子仅供演示（DEMO ONLY）—— 生产应用必须自行生成自己的 Ed25519 密钥。客户端保存主机公钥与 `peer_id` 的副本（`examples/connect-demo/client/src/identities.ts`）：

```ts
/** Public key derived from {@link DEMO_SERVER_SEED} — the remote key the client trusts. */
export const DEMO_SERVER_PUBKEY = getPublicKeyEd25519(DEMO_SERVER_SEED);

/** peer_id derived from {@link DEMO_SERVER_PUBKEY} — the client's allowlist entry. */
export const DEMO_SERVER_PEER_ID = derivePeerIdFromEd25519Pubkey(
  DEMO_SERVER_PUBKEY,
);
```

在真实集成中，密钥分发由传输 adapter 方负责：你带外获取主机的公钥并固定它，就像 demo 固定自己的常量一样。

- `allowlist` —— 该 adapter 接受的对等节点标识；远端 `peer_id` 必须列入。拨号失败 —— 密钥错误、allowlist 缺项、握手被拒 —— 会让 `connectRemoteAdapter` promise 拒绝，且不存在 adapter 实例。

在**终端 2** 运行客户端：

```bash
node examples/connect-demo/client/dist/main.js --url ws://127.0.0.1:8787
```

拨号建立，CLI 打印会话：

```text
SPOKE connect demo — third-party client
  dialing ws://127.0.0.1:8787 as 12D3KooWM82bDYYgzgXaayHDdVciFe3bGvJ69qHnbSztNUJ933VQ
  remote peer: 12D3KooWNm5t4HypYRmiC5v9CD2TnPKrJh2J8TcfJ2gPhA7L8TiZ (demo-inference-host)
    capabilities: spoke-baseline, tools.toy_world.roll_dice, tools.toy_world.lore_lookup, l2-computable, l5-fork
    namespaces:   demo-harbor, toy_world
```

远端 peer id 与主机打印的 `peer_id` 一致，manifest 就是你在第 1 节认识的服务器 manifest —— adapter 以 `adapter.remoteManifest` 暴露它，在会话建立时缓存。

## 5. 调用 port 方法

adapter 实现异步 `BaselinePorts` 六族，因此你可以调用 knowledge、relation、scope、finding、rule 与 host-manifest 方法 —— 就像对等节点在本地一样。demo 客户端以乐观并发（OCC）演练 knowledge 族（`examples/connect-demo/client/src/main.ts`）：

```ts
  // Step 1 — capability manifest (cached at establish, no round-trip).
  const serverManifest = adapter.remoteManifest;

  // Step 2 — put → get round-trip with OCC: create, then compare-and-swap.
  const created = requireOk(
    await adapter.putKnowledgeEntry(SUBMITTED_ENTRY, null),
  );
  if (created.revision === undefined) {
    throw new Error("demo client: created entry has no revision");
  }
  const updated = requireOk(
    await adapter.putKnowledgeEntry(
      { ...SUBMITTED_ENTRY, status: "confirmed" },
      created.revision,
    ),
  );
  const fetched = requireOk(
    await adapter.getKnowledgeEntry(SUBMITTED_ENTRY.entry_id),
  );

  // Step 3 — list: seed corpus + submitted entry + engine-derived artifacts.
  const listed = requireOk(
    await adapter.listKnowledgeEntries({ scope_id: DEMO_SCOPE_ID }),
  );

  // Step 4 — findings round-trip.
  const findings = requireOk(await adapter.putFindings([SUBMITTED_FINDING]));

  // Step 5 — peer host manifests (the demo host knows no peers).
  const peerManifests = requireOk(
    await adapter.listPeerHostCapabilityManifests(),
  );
```

这里有两件事值得注意。

第一，`putKnowledgeEntry` 是有条件的：第二个参数是期望的基础 revision。`null` 表示**创建** —— 条目必须尚不存在；数字表示**比较并交换** —— 存储的当前 revision 必须等于它。第一次 put 以 revision 1 创建条目；第二次 put 传入 `created.revision`，把条目更新到 revision 2（并把状态翻转为 `confirmed`）。revision 归存储所有 —— 由主机分配，绝不是调用方。提交的条目是一个普通 `KnowledgeEntry`：

```ts
const SUBMITTED_ENTRY: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "demo-harbor/item/compass",
  entry_type: "item",
  canonical_name: "Compass",
  status: "provisional",
  body: { summary: "A brass compass." },
  extensions: {},
};
```

第二，每次 port 调用都结算为一个 `SpokeResult` —— 一个可辨识的 `{ ok: true, value }` / `{ ok: false, code, message }` 联合 —— 而不是抛出异常。demo 用一个在拒绝时响亮失败的辅助函数解包（`examples/connect-demo/client/src/main.ts`）：

```ts
/** Unwrap a port-call result or fail the demo loudly (no silent fallbacks). */
function requireOk<T>(result: AnySpokeResult<T>): T {
  if (!result.ok) {
    throw new Error(
      `demo client: port call rejected (${result.code}): ${result.message}`,
    );
  }
  return result.value;
}
```

`getHostCapabilityManifest` 是特例：它是会话缓存，在建立时从签名握手提供 —— 无往返。这正是客户端读取 `adapter.remoteManifest` 而不是调用该 port 的原因。

## 6. 观察模拟推理

主机的引擎监视存储，并在每次被接受的变更之后推导自己的产物。看 CLI 在运行结束时打印的列表输出：

```text
  listKnowledgeEntries → 5 entries (demo-harbor/character/mira, demo-harbor/location/harbor, derived/world-digest, demo-harbor/item/compass, demo-harbor/artifact/dice-roll)
```

前两个条目是种子语料；`demo-harbor/item/compass` 是你 put 的条目；`derived/world-digest` 是引擎的；`demo-harbor/artifact/dice-roll` 是编排的掷骰回填（见[暴露并调用远程工具](/zh/how-to/connect-remote-tools)）。每次被接受的 put 都会重跑推导，构建一个保留 id 的 KnowledgeEntry（`examples/connect-demo/server/src/engine/mock-engine.ts`）：

```ts
    const digest: KnowledgeEntry = {
      schema_version: 1,
      entry_id: DERIVED_WORLD_DIGEST_ENTRY_ID,
      entry_type: "note",
      canonical_name: "World Digest",
      status: "confirmed",
      body: {
        summary: `Digest of ${userEntries.length} knowledge entries in demo-harbor.`,
        computable: {
          entry_type_counts: entryTypeCounts,
          entry_ids_sorted: sortedIds,
        },
      },
      revision: this.derivationCount,
      extensions: {},
    };
```

demo 流程之后，摘要读取为：

```json
{
  "schema_version": 1,
  "entry_id": "derived/world-digest",
  "entry_type": "note",
  "canonical_name": "World Digest",
  "status": "confirmed",
  "body": {
    "summary": "Digest of 4 knowledge entries in demo-harbor.",
    "computable": {
      "entry_type_counts": {
        "character": 1,
        "location": 1,
        "item": 1,
        "artifact": 1
      },
      "entry_ids_sorted": [
        "demo-harbor/artifact/dice-roll",
        "demo-harbor/character/mira",
        "demo-harbor/item/compass",
        "demo-harbor/location/harbor"
      ]
    }
  },
  "revision": 4,
  "extensions": {}
}
```

摘要的 `revision` 等于推导计数 —— 每次被接受的 put 都会前进，因此该产物是用户历史的稳定函数。`derived/` id 命名空间是保留的：用户向其中的 put 会被拒绝。这就是真实推理主机的输出经过同一个 `BaselinePorts` 面的样子：派生的知识出现在普通列表与读取中，形状上与用户数据无异。

## 7. 处理错误

有两类失败值得注意，它们以不同的方式呈现。

**拨号失败发生在 adapter 存在之前。** 如果主机拒绝你的握手 —— allowlist 错误、密钥错误、nonce 重放 —— `connectRemoteAdapter` 会拒绝，你没有任何 adapter 可关闭。demo 端到端地证明了 allowlist 路径：第三个身份 `DEMO_STRANGER_SEED` 信任服务器，但不在服务器的 allowlist 上。服务端 allowlist 检查使握手失败并在拨号中途关闭 socket，于是客户端的拨号因连接丢失而快速失败（`examples/connect-demo/client/tests/e2e.test.ts`）：

```ts
  it("rejects a dial from a non-allowlisted stranger identity", async () => {
    const transport = new WsTransport(server.url);
    transports.push(transport);

    // The stranger's OWN allowlist trusts the server, so the dial is
    // attempted; the SERVER-side allowlist rejects the hello and closes the
    // socket, failing the dial fast — no session is established. The
    // rejection is the handshake's connection loss (the server hung up
    // mid-dial), not a bare any-error assertion.
    await expect(
      connectRemoteAdapter({
        transport,
        localIdentity: { seed: DEMO_STRANGER_SEED },
        localManifest: DEMO_CLIENT_MANIFEST,
        remotePubkey: DEMO_SERVER_PUBKEY,
        allowlist: [DEMO_SERVER_PEER_ID],
      }),
    ).rejects.toThrow(/ws connection closed/);

    transport.close();
  });
```

**port 调用失败结算为 `SpokeResult` 拒绝。** 在已建立的会话上，每次 port 调用要么以 `ok` 解析，要么以 `{ ok: false, code, message, details }` 拒绝 —— 你的代码按 `result.ok` 分支，或用 `requireOk` 这样的辅助函数解包。拒绝携带线上码（对已存在 id 的创建是 `REVISION_CONFLICT`，基础 revision 过期是 `STORED_REVISION_STALE`，保留的 `derived/` id 是 `INVALID_INPUT`，……），对于基础设施失败还有 `details.kind` 告诉你哪一层失败，例如 `transport`（I/O）、`session_closed`（连接丢失 —— 停止主机，观察进行中的调用拒绝）、`timeout`（仅该调用；会话保持可用）等。完整失败表见[通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter)。

这就是完整的错误面：拨号在 adapter 存在之前拒绝，port 调用在之后拒绝 —— 两类失败，各经一个通道呈现。

## 8. 驱动可选 port 族

demo 的会话在 `spoke-baseline` 之外还携带两个可选族 —— `l2-computable`（`project` / `compute` 会话）与 `l5-fork`（分支时间线查询）。双方 manifest 都声明了它们（第 1 节与第 4 节），因此双方协商了它们，响应方的 dispatch gate 放行 `port.computable.*` / `port.fork.*` op。

### 服务 —— 主机侧

服务器经与基线相同的 `ports` 接缝服务这些族：`DemoOrchestrator` 实现组合 `FullPorts` 契约，`MockEngine` 拥有确定性状态。`project` 从请求的静态状态物化会话的可计算视图并记录该会话；`compute` 把请求的增量合并进会话视图，并且当 `settle` 为 true 时把视图合并回会话的静态状态；`listForkTimelineEvents` 返回种子风暴分支时间线（`demo-harbor/fork/storm`，三个事件），与任何 scope 查询一样按作用域查询。协议层没有任何东西被重实现 —— 响应方对 provider 做族方法的结构性探测并分派目录行。

### 驱动 —— 客户端侧

`runDemoClient` 仅当它自己的 manifest 声明了这些族时才驱动可选步骤 —— 协商集是交集，因此未声明某族的服务器会响亮地拒绝，而不是被静默跳过（`examples/connect-demo/client/src/main.ts`）：

```ts
  // Steps 6-7 — optional families: drive them only when THIS client's
  // manifest declares them (the negotiated set is the intersection of both
  // manifests, so a server that does not declare a family denies loudly
  // through requireOk instead of skipping silently). The default manifest
  // declares both, so the demo flow always runs them.
  const drivesOptionalOps =
    dialManifest.capabilities.includes("l2-computable") &&
    dialManifest.capabilities.includes("l5-fork");

  // Step 6 — l2-computable round-trip: project materializes the session's
  // computable view from static state; compute applies the delta and
  // settles it back into static state (the derived state).
  let projected: ProjectSuccess | undefined;
  let computed: ComputeSuccess | undefined;
  let forkEvents: TimelineEvent[] | undefined;
  if (drivesOptionalOps) {
    const projectedResult = requireOk(
      await adapter.project({
        session_id: COMPUTABLE_SESSION_ID,
        entry_id: COMPUTABLE_ENTRY_ID,
        state: { ...PROJECT_STATE },
      }),
    );
    if ("error" in projectedResult) {
      throw new Error(
        `demo client: project answered an error branch (${projectedResult.error.code})`,
      );
    }
    projected = projectedResult;

    const computedResult = requireOk(
      await adapter.compute({
        session_id: COMPUTABLE_SESSION_ID,
        entry_id: COMPUTABLE_ENTRY_ID,
        computable: { ...COMPUTE_DELTA },
        settle: true,
      }),
    );
    if ("error" in computedResult) {
      throw new Error(
        `demo client: compute answered an error branch (${computedResult.error.code})`,
      );
    }
    computed = computedResult;

    // Step 7 — l5-fork round-trip: the seeded storm-fork timeline.
    forkEvents = requireOk(
      await adapter.listForkTimelineEvents({
        scope_id: DEMO_SCOPE_ID,
        fork_id: DEMO_STORM_FORK_ID,
      }),
    );
  }
```

CLI 在基线步骤之后立即打印可选步骤：

```text
  project            demo-harbor/location/harbor → {"ships_at_dock":3}
  compute (settle)   demo-harbor/location/harbor → {"ships_at_dock":3,"tide":"rising"} state {"ships_at_dock":3,"tide":"rising"}
  listForkTimelineEvents → 3 event(s) (demo-harbor/event/storm-landfall, demo-harbor/event/harbor-evacuation, demo-harbor/event/compass-secured)
```

`project` 返回物化视图（`{ ships_at_dock: 3 }`）；`compute` 应用增量 `{ tide: "rising" }` 并结算 —— 结算后的视图与派生的静态状态都读作 `{ ships_at_dock: 3, tide: "rising" }`；分支时间线逐字来自种子语料。

### 拒绝 —— 未声明的能力

拒绝路径与任何分派拒绝同一条 fail-closed 行：只有一侧声明的族不在协商集中，于是响应方的 gate 以线上码 `op_unsupported` 应答，客户端把它映射为带 `details.wire_code` 的 `CAPABILITY_PORT_MISSING` 拒绝。e2e 用一个 manifest 省略 `l2-computable` 的服务器变体证明它 —— 断言本身即客户端侧的映射拒绝（`examples/connect-demo/client/tests/e2e.test.ts`）：

```ts
      const result = await adapter.compute({
        session_id: "demo-session/deny-negative",
        entry_id: "demo-harbor/location/harbor",
        computable: { tide: "rising" },
        settle: true,
      });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe("CAPABILITY_PORT_MISSING");
        expect(result.details?.wire_code).toBe("op_unsupported");
      }
```

拒绝绝不静默成功：调用方观察到拒绝，引擎中也没有任何回填。

同样的可选面也存在于原生绑定上 —— `RemoteAdapterFFI.project` / `.compute` / `.list_fork_timeline_events`，以及响应方可选的带外回调 `PortsHandler` —— 见[从原生绑定使用 RemoteAdapter](/zh/how-to/remote-adapter-native-binding)。完整目录与服务契约见[可选 port 族](/zh/reference/connect#可选-port-族)。

## 你现在掌握了

- 从外部看 connect 主机是什么样：签名握手响应方背后的 `BaselinePorts` adapter，在 manifest 中通告能力与命名空间。
- 消息导向的 `Transport` 接缝：每次 `send`/`recv` 一个信封，`recv` 在关闭时拒绝，`close` 幂等 —— 以及它的完整 WebSocket 实现。
- 如何用 `connectRemoteAdapter` 拨号：`transport`、`localIdentity`、`localManifest`、`remotePubkey`，以及 fail-closed 的 `allowlist`。
- `BaselinePorts` 调用模式：带乐观并发（OCC）的条件 put、`SpokeResult` 返回，以及由会话缓存提供的 `getHostCapabilityManifest`。
- 主机侧推理出现在哪里：带保留 id 的派生产物出现在普通列表中。
- 失败如何呈现：adapter 存在之前的拨号拒绝，以及之后带 `details.kind` 的 `SpokeResult` 拒绝。
- 可选族如何端到端流转：在双方 manifest 中声明它们，经响应方 `ports` provider 服务它们，用 `project` / `compute` / `listForkTimelineEvents` 驱动它们，并在某族未协商时观察 fail-closed 拒绝（`CAPABILITY_PORT_MISSING`，`wire_code: "op_unsupported"`）。

## 下一步

- [通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter) —— 面向任务的对偶页面：完整选项表、并发规则与错误映射。
- [跨多个对等节点路由](/zh/how-to/multi-peer-routing) —— 在同一个 `BaselinePorts` 面之后组合多个已建立的 adapter。
- [connect 线上参考](/zh/reference/connect) —— 信封字段表、信封认证与 port-method ops 目录。
- [从原生绑定使用 RemoteAdapter](/zh/how-to/remote-adapter-native-binding) —— 相同的 adapter 生命周期、可选 port 面与 FFI 上的 `PortsHandler`。
