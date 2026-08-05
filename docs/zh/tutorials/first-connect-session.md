---
title: 开启你的首个 connect 会话
---

# 开启你的首个 connect 会话（Open your first connect session）

本教程端到端地建立一个 SPOKE connect 会话：推导你的 `peer_id`、签署握手（hello）、对照 allowlist 校验对方的签名握手，并带关联（correlation）地调用一个 op。它使用 TypeScript **语言原生客户端**（`@42ch/spoke-connect`）对接本地对端，并把对端一侧指向 Rust 参考 crate。

connect 是面向跨进程 SPOKE 主机的可选交互信封族（`spoke-connect` 能力标志）。建议先完成[安装并创建你的第一条 KnowledgeEntry](/zh/tutorials/install-and-first-entry) —— 本教程在数据/ops 叙事之上构建身份与会话概念。

## 1. 安装客户端

```bash
pnpm add @42ch/spoke-connect@X.Y.Z
```

`@42ch/spoke-connect` 提供两个入口：

- **`.`** —— 同构核心：身份推导、Ed25519 密码学、RFC 8785 JCS 规范化，以及纯会话核心规则（allowlist、nonce、sequence、correlation、dispatch gate）。
- **`./node`** —— Node 版 `connectClient`，负责拨号 WebSocket 并完成完整握手。

## 2. 推导你的对等节点身份

每个 connect 主机都有一对 Ed25519 密钥。线上的 `peer_id`（对等节点标识）由 32 字节公钥推导而来 —— libp2p `PublicKey` protobuf 的身份 multihash，再经 base58btc 编码。该推导在 TypeScript 客户端、Rust 参考实现与全部原生绑定之间字节一致（由共享 golden vectors 锁定）。

```ts
import { derivePeerIdFromEd25519Pubkey, getPublicKeyEd25519 } from "@42ch/spoke-connect";

const seed = new TextEncoder().encode("..."); // 32-byte Ed25519 seed
const publicKey = getPublicKeyEd25519(seed);
const peerId = derivePeerIdFromEd25519Pubkey(publicKey);

console.log(peerId); // base58btc, e.g. 12D3KooW...
```

`peer_id` 是网络信任根 —— 与 manifest 内携带的咨询性 `host_id`（主机标识）不同。

## 3. 签署并校验握手

握手是一个已签名的 `ConnectHello`：`{protocol_version, peer_id, nonce, host}` 对象（发起方 hello）—— 或 `{protocol_version, peer_id, nonce, host, peer_nonce}`（响应方 hello，`peer_nonce` = 发起方的 nonce，即拨号绑定）—— 先经 RFC 8785 JCS 规范化（[RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)），再用 Ed25519 签名，原始签名以无填充 base64url 编码（[RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648)）。

```ts
import { generateNonce, signHelloEd25519, verifyHelloEd25519 } from "@42ch/spoke-connect";
import type { HostCapabilityManifest } from "@42ch/spoke-schemas";

const manifest: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "host_tutorial",
  roles: ["input-source"],
  capabilities: ["spoke-baseline"],
  namespaces: ["tutorial"],
  extensions: {},
};

const nonce = generateNonce(); // 16 CSPRNG 字节，base64url —— 满足 16 字符线上下限
const hello = await signHelloEd25519(seed, nonce, manifest);

// 接收侧：对照发送方的公钥与其推导出的 peer_id 校验 ——
// 一个推导出不同 peer_id 的密钥无法为该对等节点的身份作证。
await verifyHelloEd25519(remotePubkey, remotePeerId, hello);
```

nonce 按发送方单次使用：接收方在 `NonceStore` 中记录每个已接受的 `(peer_id, nonce)` 对，并拒绝重放。

**响应方**（收到 hello 的一方）在签署自己的 hello 时带上发起方的 nonce —— 即拨号绑定。发起方在验证时传入自己的 nonce，因此捕获的响应方 hello 无法重放进新的拨号：

```ts
// 响应方：把发起方的 nonce 回显进签名对象（5 个字段）。
const responderHello = await signHelloEd25519(seed, generateNonce(), manifest, receivedHello.nonce);

// 发起方：断言响应方的 peer_nonce 等于我们自己的 nonce。
await verifyHelloEd25519(remotePubkey, remotePeerId, responderHello, ourNonce);
```

## 4. 配置 allowlist

准入是 fail-closed 的：空 allowlist 拒绝所有对端。接收主机只接受认证后的远端 `peer_id` 出现在列表中的连接。

```ts
import { isAllowlisted, NonceStore } from "@42ch/spoke-connect";

const allowlist = [remotePeerId];
if (!isAllowlisted(allowlist, remotePeerId)) {
  throw new Error(`peer ${remotePeerId} is not allowlisted`);
}

const nonceStore = new NonceStore();
nonceStore.checkAndRecord(remotePeerId, hello.nonce); // 重放时返回 false
```

## 5. sequence 与 correlation

每个会话维护从 0 开始的按方向单调 `sequence` 计数器。每次 invoke 附带 `request_id`；响应必须回显 `session_id`、`sequence` 与 `request_id`，否则关联校验失败。

```ts
import { OutboundSequence, checkResponseCorrelation, correlationFromRequest, correlationFromResponse } from "@42ch/spoke-connect";

const outbound = new OutboundSequence();
const request = {
  session_id: "sess_1",
  sequence: outbound.allocate(), // 首次调用 → 0
  request_id: crypto.randomUUID(),
  op: "check",
  payload: { scope: { scope_id: "book-harbor" } },
  extensions: {},
};

// 响应到达时：
checkResponseCorrelation(correlationFromRequest(request), correlationFromResponse(response));
```

## 6. 完整会话：`connectClient`

`connectClient`（位于 `./node` 子路径）拨号一个 WebSocket，完成签名握手交换，校验会话快照（对端绑定、`initial_sequence` 为 0），并按 `request_id` 路由带关联的 invoke：

```ts
import { derivePeerIdFromEd25519Pubkey } from "@42ch/spoke-connect";
import { connectClient } from "@42ch/spoke-connect/node";

const remotePubkey = /* 对端的 32 字节 Ed25519 公钥 */;

const client = await connectClient({
  url: "ws://127.0.0.1:8080",
  identity: { seed },
  manifest,
  remotePubkey,
  allowlist: [derivePeerIdFromEd25519Pubkey(remotePubkey)],
});

const response = await client.invoke("check", { scope: { scope_id: "book-harbor" } });
client.close();
```

当远端 `peer_id` 不在 allowlist 中时，客户端会在握手开始前拒绝；当快照中的对端 id 与已认证握手不符时，客户端会拒绝该会话。

## 7. 对端一侧

`connectClient` 可连接任意在有序可靠流上使用 connect 信封族的 SPOKE 主机。**Rust 参考 crate**（crates.io 上的 `spoke-connect`）是参考主机实现 —— 它把信封族映射到 rust-libp2p（noise、yamux、request-response），并演示一个两节点会话：一个节点拨号另一个节点、交换签名握手、然后调用 `check`：

```bash
cargo add spoke-connect@X.Y.Z
cargo run -p spoke-connect --example two_node_usage
```

编译好的示例源码（[`examples/two_node_usage.rs`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/examples/two_node_usage.rs)）展示两侧：`SpokeConnectNode::start` 携带 `peer_allowlist` 与本地 manifest，然后 `connect(addr)` 与 `session.invoke("check", payload)` —— 与 TypeScript 客户端实现相同的会话规则。crate README（[`crates/spoke-connect/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md)）记载了完整流程，包括 capability-token 提权鉴权。

## 你现在掌握了

- 从 Ed25519 公钥推导 `peer_id`，以及它为何是信任根。
- 签名握手（`spoke-connect-hello-jcs-v1`）：对 `{protocol_version, peer_id, nonce, host}`（发起方）/ 加 `peer_nonce`（响应方）做 JCS，Ed25519 签名，base64url，以及拒绝重放响应方 hello 的拨号绑定。
- Fail-closed 的 allowlist 准入与单次使用 nonce 重放保护。
- 按会话的 sequence 与 `request_id` 关联。

## 下一步

- [从 TypeScript 客户端连接](/zh/how-to/connect-ts-client) —— 完整客户端面、浏览器 vs Node，以及核心辅助函数。
- [从原生绑定连接](/zh/how-to/connect-native-bindings) —— 从 C#、Kotlin、Swift、Go 或 Python 使用同一会话核心。
- [connect 线上参考](/zh/reference/connect) —— 信封字段表与身份绑定规则。
