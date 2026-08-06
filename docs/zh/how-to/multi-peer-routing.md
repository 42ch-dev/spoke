---
title: 跨多个对等节点路由
---

# 跨多个对等节点路由（Route across multiple peers）

**多对等节点路由器（multi-peer router）**（TypeScript 中为 `connectMultiPeerRouter`，Rust 中为 `connect_multi_peer_router`）位于你的各对等节点 connect 会话与 `orchestrate*` 调用之间。你注册本节点已拨号（dial）的对等节点，每一次调用 —— TypeScript 的 `orchestrateUpsert(router, req)`、Rust 的 `orchestrate_upsert(&router, req)` —— 都会被路由到恰好一个已注册对等节点：请求载荷只携带操作本身，由路由器选择对等节点。`orchestrate*` 入口与单对等节点 adapter 共用同一 `BaselinePorts` 面，因此路由器可直接接入同一套编排调用。

## 1. 拨号并注册对等节点

路由器启动时带有零个对等节点。你自行拨号每个对等节点的 `RemoteAdapter`（握手签名、allowlist、会话建立 —— 完整拨号契约见[通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter)），然后注册已建立的 adapter。路由器在注册时存储该 adapter 并缓存对等节点的 `HostCapabilityManifest` —— 即来自已认证握手（hello）的 `host` 字段。

```ts
import { connectMultiPeerRouter } from "@42ch/spoke-connect/remote";

// 路由器启动时带有零个对等节点；注册你已拨号的 adapter。
const router = connectMultiPeerRouter({ hostId: "storefront-router" });

const north = await connectRemoteAdapter({ /* 拨号选项 */ });
const south = await connectRemoteAdapter({ /* 拨号选项 */ });

const northId = router.registerPeer(north); // 返回远端 peer_id
router.registerPeer(south);                 // 对 peer_id 幂等

router.listPeers();      // ["12D3KooW…", "12D3KooW…"] —— 注册顺序
router.unregisterPeer(northId); // 移出选择；adapter 保持开启
```

```rust
use spoke_connect::remote::{
    connect_multi_peer_router, connect_remote_adapter, MultiPeerRouterOptions,
};

let router = connect_multi_peer_router(MultiPeerRouterOptions {
    host_id: Some("storefront-router".into()),
});

let north = connect_remote_adapter(/* 拨号选项 */)?;
let south = connect_remote_adapter(/* 拨号选项 */)?;

let north_id = router.register_peer(north)?; // Ok(peer_id)
router.register_peer(south)?;                // 对 peer_id 幂等

router.list_peers();        // 注册顺序
router.unregister_peer(&north_id); // 移出选择；adapter 保持开启
```

| 操作 | 行为 |
|------|------|
| `registerPeer(adapter)` / `register_peer(&adapter)` | 接受已建立的 adapter，返回其 `peer_id`，缓存其 manifest。对同一 `peer_id` 重复注册会替换已存储的 adapter（幂等）。adapter 必须拥有已建立的会话 —— 先拨号。 |
| `unregisterPeer(peerId)` / `unregister_peer(&peer_id)` | 将该对等节点移出选择。对未注册过的 `peer_id`，注册表保持不变。adapter 的生命周期由消费方持有 —— 路由器保持 adapter 开启。 |
| `listPeers()` / `list_peers()` | 已注册的 `peer_id`，按注册顺序。 |

已注册对等节点的会话若离开 `Established` 状态（例如进入 `Closed`、`Disconnected` 或 `Handshaking`），将在下一次调用时被移出候选集。注册表会保留该对等节点，直到你调用 `unregisterPeer`；排除是反应式的，基于 adapter 报告的会话状态。

## 2. 选择输入

选择过程读取每个已注册对等节点缓存的 `HostCapabilityManifest`，并与操作进行匹配。四个 manifest 字段驱动选择，分为硬过滤器与一个软偏好：

| 输入 | 来源 | 过滤器类型 | 在选择中的作用 |
|------|------|------------|----------------|
| `capabilities` | 对等节点的 `capabilities[]` | **硬门禁** | 对等节点必须声明操作所需的能力 |
| `namespaces` | 对等节点的 `namespaces[]` | **硬门禁** | 请求的命名空间（取自载荷 `Scope`）必须精确匹配 |
| `roles` | 对等节点的 `roles[]` | **软偏好** | 拥有操作首选角色（preferred role）的对等节点优先；缺少该角色但有能力的对等节点仍可被选中 |
| `authority.scope_key` | 对等节点的 `authority.scope_key` | **双方都声明时为硬门禁** | 对等节点作用域键与请求作用域键精确匹配 |

每个操作族映射到一个必需能力 —— 完整表格见线上参考中的[能力词汇（Capability vocabulary）](/zh/reference/connect#能力词汇-capability-vocabulary)。

请求的命名空间在操作携带 `Scope` 时从载荷推导（例如 `upsert-request.scope` 或 `check-request.scope`）。命名空间匹配是精确的：声明 `namespaces: ["*"]` 的对等节点声明的是字面字符串 `"*"`。当请求携带作用域键且对等节点 manifest 声明了作用域键时，二者必须精确匹配；仅一方声明时该门禁通过。

硬过滤之后，拥有操作首选角色（例如 `check` 的 `checker`、`assemble` 的 `assembler`、`project` / `compute` 的 `l2-computable`）的对等节点排在同等有能力的对等节点之前。当多个对等节点存活时，路由器选择 `peer_id` 按 UTF-8 字节序字典序最小的那个 —— 这是候选集的纯函数，因此相同的对等节点集与相同请求总会选择同一对等节点。

## 3. 失败结果

对给定对等节点集，每个路由结果都是确定性的，路由器将每个结果作为调用的返回值返回给消费方 —— 一切后续活动都始于消费方的下一次调用。

### 终结拒绝：`no_capable_peer`

当硬过滤排除所有已注册对等节点时，路由器以锁定的终结拒绝（terminal reject）拒绝：

| 字段 | 值 |
|------|-----|
| `SpokeResult` 拒绝码 | `CAPABILITY_PORT_MISSING` |
| `details.wire_code` | `no_capable_peer` |
| `details.kind` | `no_capable_peer` |

该拒绝对该请求是终结性的：路由器返回它，并且只在消费方再次调用时重新选择。注册一个满足过滤条件的对等节点，然后用新的 `request_id` 重新调用。

### 被选中的对等节点在操作中失败

当被选中对等节点的会话在委托调用期间失败时，路由器原样返回对等节点 adapter 产生的底层 `SpokeResult` 拒绝 —— `INTERNAL_ERROR`，且 `details.kind` 保持原值（例如 `transport` 或 `session_closed`）。失败始终可归因于处理该调用的对等节点。

重试由消费方负责。消费方用新的 `request_id` 重新调用；路由器针对当前对等节点集重新选择，会话离开 `Established` 的对等节点会在下一次选择时退出候选集。重试由消费方负责，是因为消费方了解每个操作的幂等语义：调用可能在传输失败之前已被对等节点执行，是否安全地重新运行由消费方决定。协议层提供会话级关联 —— `request_id`、`session_id` 与序列号 —— 而按操作的幂等决策由消费方持有。

## 4. 信封真实性是协议级的

每一个 connect 信封 —— 握手（hello）、调用（invoke）与响应 —— 都由会话对等节点的 Ed25519 密钥对 RFC 8785 JCS 规范化对象签名，并在分派前于会话核心内部完成验证。这一验证在 adapter 提供的任何有序、可靠的传输之上运行（TCP、WebSocket、yamux 或 Noise），因为真实性位于信封本身，处于传输层之上。路由器在每个对等节点会话的信封认证完成后才选择对等节点；选择是在会话核心的已认证信封流程内进行的路由决策。完整图景 —— 信封为何签名、协议版本 2 覆盖什么、混合版本对等节点如何表现 —— 见 [Connect 架构](/zh/explanation/connect#信封认证)。

## 5. 检视路由器可以触达的范围

路由器在合成对等节点集之上暴露 `HostManifestPort`，提供两种不同的聚合视图：

```ts
// 合成视图 —— 所有已连接对等节点 manifest 的并集：
const composed = await router.getHostCapabilityManifest();
// composed.value.host_id                  → 路由器自己的 host_id
// composed.value.capabilities             → 集合并集，去重
// composed.value.roles                    → 集合并集，去重
// composed.value.namespaces               → 集合并集，去重
// composed.value.extensions.router.peers  → 贡献的对等节点 peer_id，UTF-8 字节序

// 逐对等节点视图 —— 每个对等节点自己的已缓存握手 manifest：
const perPeer = await router.listPeerHostCapabilityManifests();
// perPeer.value[i].host_id → 每项一个对等节点 manifest，按 peer_id 排序
```

```rust
use spoke_operations::SpokeResult;

let composed = match router.get_host_capability_manifest().await {
    SpokeResult::Ok(manifest) => manifest,
    SpokeResult::Reject(reject) => return Err(reject), // 你的错误路径
};
// composed.host_id                  → 路由器自己的 host_id
// composed.capabilities             → 集合并集，去重
// composed.roles                    → 集合并集，去重
// composed.namespaces               → 集合并集，去重
// composed.extensions["router"]["peers"] → 贡献的对等节点 peer_id，UTF-8 字节序

let per_peer = match router.list_peer_host_capability_manifests().await {
    SpokeResult::Ok(manifests) => manifests,
    SpokeResult::Reject(reject) => return Err(reject), // 你的错误路径
};
// per_peer[i].host_id → 每项一个对等节点 manifest，按 peer_id 排序
```

| 视图 | 形状 | 用途 |
|------|------|------|
| `getHostCapabilityManifest` | 一个合成 manifest：路由器自己的 `host_id`、已连接对等节点 `capabilities` / `roles` / `namespaces` 的集合并集，以及按 UTF-8 字节序字典序列出贡献 `peer_id` 的 `extensions.router.peers`。合成视图只呈现能力、角色与命名空间并集；需要某个对等节点 `authority.scope_key` 的消费方读取逐对等节点视图。 | 检视：「这个节点能触达什么？」 |
| `listPeerHostCapabilityManifests` | 每个已连接对等节点一个 manifest（各对等节点自己的已缓存握手 manifest），按 `peer_id` 的 UTF-8 字节序字典序排列。注册表为空的路由器返回 `[]`。 | 逐对等节点的 authority 与 manifest 细节 |

路由读取每个对等节点自己的已缓存 manifest；合成视图是一个检视面，与选择过程保持分离。

## 下一步

- [通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter) —— 每个已注册对等节点 adapter 的拨号契约。
- [实现 Adapter](/zh/how-to/implement-adapter) —— 路由器委托到的逐对等节点 port 面。
- [编排操作](/zh/how-to/orchestrate-ops) —— 流经路由器的 `orchestrate*` 调用。
- [TypeScript 客户端](/zh/how-to/connect-ts-client) —— 拨号每个对等节点会话。
- [connect 线上参考](/zh/reference/connect) —— 信封签名、身份绑定与会话核心规则。
