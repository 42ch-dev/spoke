---
title: 从原生绑定使用 RemoteAdapter
---

# 从原生绑定使用 RemoteAdapter（Use RemoteAdapter from a native binding）

**原生绑定（native bindings）**把远程 Adapter 契约暴露为同步 FFI 面：你的宿主语言实现一个消息导向 `Transport`（传输接口），经它拨号，然后调用与 Rust 参考实现、TypeScript 语言原生客户端相同的 `BaselinePorts` 方法。共享库拥有一个进程级 tokio 运行时；每个导出调用都是该运行时之上的同步 block-on-async（同步阻塞执行异步调用），会话核心始终封装在 Rust 侧 —— 握手签名/校验、allowlist、nonce 单次使用、sequence、关联校验与信封认证全部在绑定内部运行，永不进入你的宿主代码。

导出的对象是 `RemoteAdapterFFI`（单对等节点）与 `MultiPeerRouterFFI`（多对等节点路由）。本页以 Python 绑定走完完整流程；C#、Go、Kotlin 与 Swift 存在相同的面，只是使用各语言惯用名称（见[符号对照表](#各绑定符号对照表)）。

## 1. 实现回调 `Transport`

绑定实现消息导向的 `Transport` 接口：

| 方法 | 行为 |
|------|------|
| `send(envelope)` | 接受恰好一个 connect 信封的字节 |
| `recv()` | 返回下一个入站信封；阻塞直到一个信封到达或连接关闭 |
| `close()` | 释放资源；幂等 |

crate 导出 `loopback_transport_pair()` —— 一个内存中的客户端/服务端对 —— 让你无需网络即可跑通完整流程。参考回环（loopback）smoke 把一个 `LoopbackTransport` 端交给仅测试用的 smoke host，从另一端通过回调 transport 驱动 adapter：

```python
import spoke_connect

class LoopbackCallbackTransport:
    """委托给回环对客户端端的回调 transport。"""

    def __init__(self, inner: spoke_connect.LoopbackTransport) -> None:
        self._inner = inner

    def send(self, envelope: bytes) -> None:
        self._inner.send(envelope)

    def recv(self) -> bytes:
        return self._inner.recv()

    def close(self) -> None:
        self._inner.close()
```

真实部署中，用同样的三个方法覆盖你的承载载体 —— socket、WebSocket 或消息通道 —— 每次 `send` / `recv` 调用投递恰好一个 connect 信封。adapter 在你的传输之上处理分帧；字节流载体自行界定信封（长度前缀或等价方式）。

## 2. 拨号并构造 `RemoteAdapterFFI`

`connect_remote_adapter_ffi` 通过你的 transport 执行拨号与握手签名交换，返回一个已建立的 adapter 句柄：

```python
pair = spoke_connect.loopback_transport_pair()

adapter = spoke_connect.connect_remote_adapter_ffi(
    LoopbackCallbackTransport(pair.client()),
    seed_client,           # 本地 Ed25519 身份种子，恰好 32 字节
    client_manifest_json,  # 本地 HostCapabilityManifest（主机能力清单），JSON
    pubkey_host,           # 远端对等节点的 Ed25519 公钥，恰好 32 字节
    [peer_id_host],        # allowlist：远端 peer_id
    None,                  # 调用超时（毫秒）；None 使用默认值
)
```

| 参数 | 含义 |
|------|------|
| `transport` | 你的 `Transport` 实现；adapter 经它收发信封 |
| `local_seed` | 本地身份的 32 字节 Ed25519 种子（原始字节） |
| `local_manifest_json` | 你的 `HostCapabilityManifest`，JSON 字符串 |
| `remote_pubkey` | 远端对等节点的 32 字节 Ed25519 公钥（原始字节） |
| `allowlist` | 该 adapter 接受的对等节点标识；远端 `peer_id` 必须列入 |
| `invoke_timeout_ms` | 可选的每次调用超时；超时只让该调用失败，会话保持可用 |

配置、握手或拨号超时失败时，构造函数在 adapter 存在之前即失败 —— 错误为 `FfiError.Dial`，`kind` 为 `config`、`handshake` 或 `timeout`。你的 transport 自身的失败映射为 `TransportError`（连接关闭时为 `Closed`，I/O 错误时为 `Io`）。

## 3. 调用 port 方法

port 方法接收 JSON 载荷，返回 JSON 字符串。在已建立的会话上做一个 `put` / `get` 往返：

```python
entry_json = json.dumps({
    "schema_version": 1,
    "entry_id": entry_id,
    "entry_type": "character",
    "canonical_name": "Ada",
    "status": "provisional",
    "body": {"summary": "Upserted over the connect session"},
    "extensions": {},
})

put_json = adapter.put_knowledge_entry(entry_json, None)
get_json = adapter.get_knowledge_entry(entry_id)
```

每个 `BaselinePorts` 族映射为一个方法，形状相同（JSON 进 / JSON 出）：`get_host_capability_manifest`、`get_relation` / `put_relation`、`list_knowledge_entries` / `list_timeline_events`、`put_findings`、`list_rules` 与 `list_peer_host_capability_manifests`。调用路径的失败以 `FfiError.Rejected` 呈现，`SpokeResult` 码原样保留，并在映射定义的场景携带 `kind` 与 `wire_code`（例如 `INTERNAL_ERROR` 且 `kind = "transport"`，或 `CAPABILITY_PORT_MISSING` 且 `wire_code = "no_capable_peer"`）。

同一 adapter 上的并发调用被允许；响应按 `request_id` 解复用，可能乱序到达。

## 4. 读取会话信息

adapter 暴露只读的会话元数据：

```python
adapter.state()            # "Established"、"Handshaking"、"Closed"、……
adapter.session_id()       # 会话建立后的会话 id
adapter.remote_peer_id()   # 已认证的远端 peer_id
adapter.remote_manifest()  # 远端对等节点的 HostCapabilityManifest，JSON
```

`session_id` / `remote_peer_id` / `remote_manifest` 在会话建立后填充；会话信息来自已认证握手与会话核心 —— 无需额外往返。

## 5. 用 `MultiPeerRouterFFI` 跨多个对等节点路由

`new_multi_peer_router_ffi()` 返回一个空路由器。拨号每个对等节点的 `RemoteAdapterFFI`（第 2 步），注册已建立的句柄，之后每次 port 调用都会路由到恰好一个有能力的对等节点：

```python
router = spoke_connect.new_multi_peer_router_ffi()

north_id = router.register_peer(north_adapter)  # 返回远端 peer_id
router.register_peer(south_adapter)             # 对 peer_id 幂等

router.list_peers()       # 已注册的 peer_id，注册顺序
router.unregister_peer(north_id)  # 移出选择；adapter 保持开启

result_json = router.get_knowledge_entry(entry_id)  # 路由到有能力的对等节点
```

选择过程读取每个已注册对等节点缓存的 `HostCapabilityManifest` —— 对操作必需能力与精确命名空间的硬门禁、软角色偏好，以及确定性的最小 `peer_id` 决胜规则。当没有已注册对等节点通过硬门禁时，调用以 `CAPABILITY_PORT_MISSING` 与 `wire_code = "no_capable_peer"` 拒绝；注册一个满足条件的对等节点，然后用新的 `request_id` 重新调用。路由器还暴露合成视图与逐对等节点视图的 `HostManifestPort`（`get_host_capability_manifest` 与 `list_peer_host_capability_manifests`）。完整选择契约见[跨多个对等节点路由](/zh/how-to/multi-peer-routing)。

## 6. 各绑定符号对照表

| 面 | C# | Go | Kotlin | Python | Swift |
|---------|----|----|--------|--------|-------|
| 拨号并构造 | `ConnectRemoteAdapterFfi(...)` | `ConnectRemoteAdapterFfi(...)` | `connectRemoteAdapterFfi(...)` | `connect_remote_adapter_ffi(...)` | `connectRemoteAdapterFfi(...)` |
| Adapter 对象 | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` |
| 路由器构造 | `NewMultiPeerRouterFfi()` | `NewMultiPeerRouterFfi()` | `newMultiPeerRouterFfi()` | `new_multi_peer_router_ffi()` | `newMultiPeerRouterFfi()` |
| 路由器对象 | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` |
| port 方法 | PascalCase（`GetKnowledgeEntry`） | PascalCase（`GetKnowledgeEntry`） | camelCase（`getKnowledgeEntry`） | snake_case（`get_knowledge_entry`） | camelCase（`getKnowledgeEntry`） |

每个绑定都携带相同的面：golden-parity smoke 断言字节级一致的会话核心行为，回环 smoke 从各宿主侧驱动回调 `Transport` + `RemoteAdapterFFI` 流程。

## 下一步

- [跨多个对等节点路由](/zh/how-to/multi-peer-routing) —— `MultiPeerRouterFFI` 背后的选择契约。
- [从原生绑定连接](/zh/how-to/connect-native-bindings) —— 安装并认证每个绑定渠道。
- [从 TypeScript 客户端连接](/zh/how-to/connect-ts-client) —— 共享同一套会话核心规则的语言原生客户端。
- [connect 线上参考](/zh/reference/connect) —— 信封签名、身份绑定与会话核心规则。
