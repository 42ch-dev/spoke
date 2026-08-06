---
title: 通过 Transport 使用 RemoteAdapter
---

# 通过 Transport 使用 RemoteAdapter（RemoteAdapter over a Transport）

**RemoteAdapter（远程适配器）**把远端 SPOKE connect 对等节点变成一个可即插即用的异步 `BaselinePorts` 面：你提供一个消息导向的 `Transport`（传输接口），adapter 经它拨号并完成签名握手交换，然后你调用同样的 port 方法 —— `getKnowledgeEntry`、`putRelation`、`listTimelineEvents` 等 —— 就像对等节点在本地一样。`orchestrateUpsert(adapter, req)` 与其它 `orchestrate*` 调用在调用方侧原样运行。

adapter 随两个软件包发布：**TypeScript** 的 `@42ch/spoke-connect/remote` 子路径，以及 `spoke-connect` 的 **Rust** `remote-adapter` cargo feature。两者都在内部强制 protocol version 2 信封认证（见 [Connect 架构](/zh/explanation/connect)中的[信封认证](/zh/explanation/connect#信封认证)）。

## 1. `Transport` 接缝

`Transport` 是由消费方实现的接缝（seam）：在 adapter 与远端对等节点之间搬运 connect 信封。它是**消息导向**的 —— 一次调用恰好搬运一个 connect 信封。

| 方法 | 契约 |
|------|------|
| `send(envelope)` | 接受恰好一个 connect 信封的字节 |
| `recv()` | 返回下一个入站信封；阻塞直到一个信封到达或连接关闭 |
| `close()` | 释放资源；幂等 |

每次调用一个信封 —— 字节流载体在把信封交给 adapter 之前应用长度前缀（或等价方式）定界。软件包附带一个内存回环对（loopback，测试用途）供测试：`loopbackTransportPair()`（TypeScript）/ `loopback_transport_pair()`（Rust）返回一条连接的两端（客户端与服务端），关闭任一端都会让对端挂起的 `recv` 失败，与真实连接断开完全一致。回环对**仅供测试**；WebSocket 与其它载体是同样的三个方法的消费方侧实现。

## 2. TypeScript —— `@42ch/spoke-connect/remote`

```bash
pnpm add @42ch/spoke-connect@X.Y.Z
```

`connectRemoteAdapter` 经你的 transport 拨号 —— 签名握手交换、allowlist 检查、会话快照校验 —— 并解析为已建立的 adapter：

```ts
import { derivePeerIdFromEd25519Pubkey, getPublicKeyEd25519 } from "@42ch/spoke-connect";
import { connectRemoteAdapter } from "@42ch/spoke-connect/remote";

const adapter = await connectRemoteAdapter({
  transport,           // your Transport implementation
  localIdentity: { seed }, // 32-byte Ed25519 seed
  localManifest,       // your HostCapabilityManifest
  remotePubkey,        // the remote peer's 32-byte Ed25519 public key
  allowlist: [derivePeerIdFromEd25519Pubkey(remotePubkey)],
  invokeTimeoutMs: 5000, // optional; bounds the handshake and each invoke
});
```

| 选项 | 含义 |
|------|------|
| `transport` | 你的 `Transport` 实现；adapter 经它收发信封 |
| `localIdentity.seed` | 本地 connect 身份的 32 字节 Ed25519 种子 |
| `localManifest` | 你的 `HostCapabilityManifest`，在签名握手中通告 |
| `remotePubkey` | 远端对等节点的 32 字节 Ed25519 公钥；远端 `peer_id` 由它推导，且必须在 allowlist 上（fail-closed） |
| `allowlist` | 该 adapter 接受的对等节点标识；远端 `peer_id` 必须列入 |
| `invokeTimeoutMs` | 可选的每次调用超时；超时只让该调用失败，会话保持可用（默认 5000） |

已建立的 adapter 暴露只读会话信息 —— `state`、`sessionId`、`remotePeerId`、`remoteManifest` —— 以及 `close()`：

```ts
adapter.state;          // "Established"
adapter.sessionId;      // the remote-assigned session id
adapter.remotePeerId;   // the authenticated remote peer_id
adapter.remoteManifest; // the remote peer's HostCapabilityManifest

adapter.close();        // releases the session; idempotent
```

拨号失败 —— 配置错误、握手拒绝或拨号超时 —— 会让 `connectRemoteAdapter` promise 拒绝；不存在 adapter 实例。

## 3. Rust —— `remote-adapter` feature

```bash
cargo add spoke-connect --features remote-adapter
cargo add async-trait
```

`Transport` trait 与 TypeScript 接口一一对应（`async fn` 方法，`Send + Sync`；`close` 默认为 no-op）：

```rust
use async_trait::async_trait;
use spoke_connect::remote::{Transport, TransportError};

#[async_trait]
impl Transport for MyTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        // deliver exactly one envelope's bytes to the peer
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        // return the next inbound envelope, or Err(TransportError::Closed)
        // when the connection closes
        Ok(Vec::new())
    }

    async fn close(&self) -> Result<(), TransportError> {
        // release resources; idempotent
        Ok(())
    }
}
```

`connect_remote_adapter` 执行拨号并返回 `Arc<RemoteAdapter>`：

```rust
use std::sync::Arc;
use spoke_connect::remote::{
    connect_remote_adapter, RemoteAdapterOptions, RemoteIdentity,
};

let adapter = connect_remote_adapter(RemoteAdapterOptions {
    transport: Arc::new(my_transport),
    local_identity: RemoteIdentity { seed: client_seed }, // 32-byte Ed25519 seed
    local_manifest: client_manifest,                      // your HostCapabilityManifest
    remote_pubkey: host_pubkey,                           // remote peer's 32-byte Ed25519 public key
    allowlist: vec![peer_id_host.into()],
    invoke_timeout_ms: None,  // None uses the default (5000 ms)
    capability_token: None,   // optional capability-token proof attached as `auth`
})?;
```

会话信息以 `Option` 形式返回（会话建立后填充）；`close()` 是同步的：

```rust
adapter.state();           // RemoteAdapterState::Established
adapter.session_id();      // Option<String>
adapter.remote_peer_id();  // Option<String>
adapter.remote_manifest(); // Option<HostCapabilityManifest>

adapter.close();
```

## 4. 调用 `BaselinePorts` 方法

adapter 实现与本地 adapter 相同的异步 `BaselinePorts` 六族 —— 知识条目、关系、作用域查询、findings、rules 与主机清单视图 —— 因此 `orchestrate*` 调用在调用方侧原样运行：

```ts
import { orchestrateUpsert } from "@42ch/spoke-operations";

const response = await orchestrateUpsert(adapter, upsertRequest);
```

```rust
use spoke_operations::{orchestrate_upsert, SpokeResult};

// SpokeResult 是普通 Ok/Reject 枚举（无 `?` 支持）—— 显式 match，与 crate 自身测试一致。
let response = match orchestrate_upsert(adapter.as_ref(), upsert_request).await {
    SpokeResult::Ok(response) => response,
    SpokeResult::Reject(reject) => return Err(reject), // 你的错误路径
};
```

你也可以直接调用 port 方法：

```ts
const put = await adapter.putKnowledgeEntry(entry, null);
const got = await adapter.getKnowledgeEntry(entry.entry_id);
```

每个 port 方法映射到一个保留的 `port.*` 产品 op（`port.knowledge.put`、`port.relation.get`、……），作为 invoke 的 `op` 携带；该映射在 adapter 内部。见线上参考中的 [Port-method ops（RemoteAdapter）](/zh/reference/connect#port-method-ops-remoteadapter)。

## 5. 并发与错误

同一已建立会话上的并发 port 调用被允许：出站 `sequence` 在发送时分配，响应按 `request_id` 解复用，完成可能乱序到达。每个挂起 invoke 携带 adapter 拥有的超时；超时只让该调用失败，会话保持可用。

port 调用结算为 `SpokeResult`；invoke 路径的失败以拒绝呈现：

| 失败 | 呈现 |
|------|------|
| 传输 I/O | `INTERNAL_ERROR` 拒绝，`details.kind = "transport"` |
| 会话关闭 / 连接丢失 | `INTERNAL_ERROR` 拒绝，`details.kind = "session_closed"`；adapter 转入 `Closed`，所有挂起 invoke 失败 |
| invoke 超时 | `INTERNAL_ERROR` 拒绝，`details.kind = "timeout"`（仅该等待者） |
| 关联不匹配 | `INTERNAL_ERROR` 拒绝，`details.kind = "correlation_mismatch"` |
| 序列耗尽 | `INTERNAL_ERROR` 拒绝，`details.kind = "sequence_exhausted"`；会话关闭 —— 开启新会话 |
| 信封认证拒绝 | `INTERNAL_ERROR` 拒绝，`details.kind` ∈ {`envelope_auth_missing`、`envelope_auth_invalid`、`envelope_auth_session_unbound`}（仅该等待者；会话保持可用） |
| 分派拒绝（`op_unsupported` / `capability_missing` 线上码） | `CAPABILITY_PORT_MISSING` 拒绝，带 `details.wire_code` |
| 未知线上码 | `INVALID_INPUT` 拒绝，带 `details.wire_code` |

拨号 / 握手 / allowlist / nonce 失败发生在 adapter 存在之前：`connectRemoteAdapter` 拒绝（TypeScript），或返回带 `Config` / `Handshake` / `Timeout` 变体的 `Err(RemoteAdapterError)`（Rust）。

## 6. 信封认证

adapter 在每条 post-hello 信封上内部强制 **protocol version 2** 逐信封认证，无需任何配置：

- 拨号在建立前校验响应方已签名的 `ConnectSession` 快照与握手身份一致；
- 每个出站 invoke 请求携带 `spoke-connect-invoke-request-jcs-v1` 签名；
- 每个入站响应先执行关联回显检查，再进行 `spoke-connect-invoke-response-jcs-v1` 校验。

信封真实性是传输层之上的协议级属性 —— 不依赖 TLS 或 Noise。见 [Connect 架构](/zh/explanation/connect#信封认证)中的信封认证，以及[线上参考](/zh/reference/connect#信封认证-protocol-version-2)中的已签名字段集。

## 7. 回环冒烟测试

仓库内回环对让你无需网络即可跑通完整流程：服务端由仓库的测试回环主机提供服务（[`tests/remote/loopback-host.ts`](https://github.com/42ch-dev/spoke/blob/main/packages/spoke-connect-ts/tests/remote/loopback-host.ts) —— 仅测试用），客户端由 `connectRemoteAdapter` 拨号：

```ts
import { derivePeerIdFromEd25519Pubkey, getPublicKeyEd25519 } from "@42ch/spoke-connect";
import { connectRemoteAdapter, loopbackTransportPair } from "@42ch/spoke-connect/remote";
// startLoopbackHost is an in-repo test fixture, NOT a package export —
// consumers write their own host (or copy this one from the linked file).
import { startLoopbackHost } from "<repo>/packages/spoke-connect-ts/tests/remote/loopback-host.ts";

const clientSeed = /* your 32-byte Ed25519 seed */;
const hostSeed = /* the remote peer's 32-byte Ed25519 seed */;

const pair = loopbackTransportPair();

// Server end (test-only): the repository's loopback host serves a local
// async BaselinePorts adapter over the server end of the pair.
const host = await startLoopbackHost({
  transport: pair.server,
  seed: hostSeed,
  clientPubkey: getPublicKeyEd25519(clientSeed),
  allowlist: [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(clientSeed))],
  adapter: toyWorldAdapter,
  hostManifest,
});

// Client end — the shipped consumer surface.
const adapter = await connectRemoteAdapter({
  transport: pair.client,
  localIdentity: { seed: clientSeed },
  localManifest,
  remotePubkey: getPublicKeyEd25519(hostSeed),
  allowlist: [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(hostSeed))],
});

const put = await adapter.putKnowledgeEntry(entry, null);
const got = await adapter.getKnowledgeEntry(entry.entry_id);

adapter.close();
host.close();
```

同样的流程是旅程的终点步骤：[从原生绑定使用 RemoteAdapter](/zh/how-to/remote-adapter-native-binding) 以带外回调 `Transport` 在 FFI 上驱动相同的握手。

## 下一步

- [集成 RemoteAdapter 连接推理主机](/zh/tutorials/integrate-remote-adapter) —— 以 demo 模拟推理主机逐步走通同一契约的学习路径。
- [跨多个对等节点路由](/zh/how-to/multi-peer-routing) —— 多对等节点路由器在同一个 `BaselinePorts` 面之后组合 N 个已注册 adapter。
- [从原生绑定使用 RemoteAdapter](/zh/how-to/remote-adapter-native-binding) —— 从 C#、Go、Kotlin、Python 或 Swift 以同步 FFI 面使用相同的 adapter 生命周期。
- [从 TypeScript 客户端连接](/zh/how-to/connect-ts-client) —— 语言原生客户端面，包括 `./remote` 入口。
- [connect 线上参考](/zh/reference/connect) —— 信封字段表、信封认证与 port-method ops 目录。
- [Connect 架构](/zh/explanation/connect) —— 会话生命周期、信封认证与能力路由。
