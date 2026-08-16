---
title: 从原生绑定使用 RemoteAdapter
---

# 从原生绑定使用 RemoteAdapter（Use RemoteAdapter from a native binding）

**原生绑定（native bindings）**把远程 Adapter 契约暴露为同步 FFI 面：你的宿主代码实现一个消息导向 `Transport`（传输接口）；adapter 经它拨号并收发信封，然后你调用与 Rust 参考实现、TypeScript 语言原生客户端相同的 `BaselinePorts` 方法。共享库拥有一个进程级 tokio 运行时；每个导出调用都是该运行时之上的同步 block-on-async（同步阻塞执行异步调用），会话核心始终封装在 Rust 侧 —— 握手签名/校验、allowlist、nonce 单次使用、sequence、关联校验与信封认证全部在绑定内部运行，而你的宿主代码提供 `Transport` 并调用 port 方法。

导出的对象是 `RemoteAdapterFFI`（单对等节点）、`MultiPeerRouterFFI`（跨多个对等节点路由）与 `ConnectResponderFFI`（接受侧），外加用于工具服务的 `ToolHandler`（工具处理器）回调。本页以 Python 绑定走完完整流程；C#、Go、Kotlin 与 Swift 存在相同的面，只是使用各语言惯用名称（见[符号对照表](#各绑定符号对照表)）。通用 RemoteAdapter 契约 —— 与 TypeScript、Rust 库共享的消息导向 `Transport` 接缝、拨号选项与错误映射 —— 见[通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter)；本页覆盖 FFI 面。

## 1. 实现回调 `Transport`

你的宿主代码实现消息导向的 `Transport` 接口：

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

真实部署中，用同样的三个方法覆盖你的承载载体 —— socket、WebSocket 或消息通道。Transport 每次 `send` / `recv` 调用投递恰好一个信封；字节流载体在把信封交给 adapter 之前应用长度前缀（或等价方式）定界。

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

配置、握手、版本不匹配或拨号超时失败时，构造函数在 adapter 存在之前失败 —— 错误为 `FfiError.Dial`，`kind` 为 `config`、`handshake`、`protocol_version_mismatch` 或 `timeout`。你的 transport 自身失败映射为 `TransportError`（连接关闭时为 `Closed`，I/O 错误时为 `Io`）。

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

`session_id` / `remote_peer_id` / `remote_manifest` 在会话建立后填充；会话信息来自已认证握手与会话核心，在建立时捕获。

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

## 6. 在 FFI 上服务并调用工具

FFI 面在两个方向携带工具契约：拨号方调用响应方经带外 `ToolHandler`（工具处理器）回调服务的工具；响应方反向调用拨号方经 `register_tool_handler` 注册的处理器服务的工具。`invoke_tool` 存在于 `RemoteAdapterFFI`、`MultiPeerRouterFFI` 与 `ConnectResponderFFI` 上；`register_tool_handler` 存在于 `RemoteAdapterFFI` 与 `ConnectResponderFFI` 上。

发现是已认证会话的属性：`remote_manifest()` 返回对等节点的 `HostCapabilityManifest`（主机能力清单）JSON，其 `tools[]` 描述符即对等节点能服务的工具：

```python
manifest = json.loads(adapter.remote_manifest())
tool_ids = [tool["capability_id"] for tool in manifest["tools"]]
```

工具 id 遵循语法 `tools.<ns>.<tool_id>`，且清单必须在 `tools[]` 与 `capabilities[]` 中同时列出该工具 —— 仅当能力字符串位于会话的协商能力（negotiated capabilities）中时，`tools.*` op 才会被分派。库侧契约见[暴露并调用远程工具](/zh/how-to/connect-remote-tools)，本面与其镜像一致。

### `ToolHandler`（工具处理器）回调

处理器接收 JSON 字符串形式的工具参数，并返回 JSON 字符串形式的结果。参考形状随 Python 回环 smoke 发布（`bindings/python/Smoke/test_loopback_remote_adapter.py`）：

```python
class _SumToolHandler:
    """Foreign-callback tool handler: sums `a` + `b` (Rust `add_handler`
    parity) and records the invocation count."""

    def __init__(self) -> None:
        self._calls = 0

    def handle(self, arguments_json: str) -> str:
        self._calls += 1
        arguments = json.loads(arguments_json)
        return json.dumps({"sum": arguments.get("a", 0) + arguments.get("b", 0)})

    def calls(self) -> int:
        return self._calls
```

从 `handle` 抛出的 `FfiError.Rejected` 作为应用拒绝原样传递给调用方 —— `code` / `message` 保留，`kind` / `wire_code` 保留在错误字段上。任何其它结果（`Dial` 错误、非契约异常或 panic）被遏制为不带 `kind` / `wire_code` 的 `INTERNAL_ERROR` 拒绝，会话继续存活。

### 在拨号方注册处理器

`RemoteAdapterFFI.register_tool_handler(capability_id, handler)` 服务响应方→拨号方的反向调用。对重复 id 的注册后者胜出，且注册从不修改清单；非 `tools.` id 以 `INVALID_INPUT` 拒绝，违规 id 在 `message` 中，零线上流量。

### 接受侧：`connect_responder_ffi`

FFI 面从不构建 listener（监听器）—— 主机产品在其自身网络栈中拥有 listen/accept（监听/接受连接）。接受流程与拨号对称：主机接受一条连接，将其包装为回调 `Transport`，并把已连接的 transport 传给 `connect_responder_ffi`：

```python
responder = spoke_connect.connect_responder_ffi(
    LoopbackCallbackTransport(pair.server()),
    seed_host,
    _tool_manifest_json("test-responder"),  # 带 tools[] 的 HostCapabilityManifest
    [peer_id_client],                      # fail-closed 拨号方 allowlist
    {peer_id_client: pubkey_client},       # peer_id -> 32 字节 Ed25519 公钥
    None,                                  # 调用超时（毫秒）；None 使用默认值
)
```

构造函数立即返回 —— 拨号方 hello 落定期间，响应方处于 `Handshaking`。在调用前以有界方式轮询 `state()` 至 `Established`；握手失败（allowlist 拒绝、hello 校验拒绝）以 `state() → "Closed"` 且 `session_id() → None` 呈现，绝不会抛出构造函数错误：

```python
import time

deadline = time.monotonic() + 5.0
while responder.state() != "Established":
    if time.monotonic() >= deadline:
        raise RuntimeError(f"handshake timeout (last: {responder.state()!r})")
    time.sleep(0.01)
```

构造函数的 `Result` 槽只携带配置校验失败 —— manifest JSON、种子长度或对等节点密钥长度 → `Dial { kind: "config" }`。FFI 响应方上 `ports` 固定缺席：发往响应方的 `port.*` 调用以文档化拒绝分支应答（`CAPABILITY_PORT_MISSING`，`wire_code: "op_unsupported"`）。

### 端到端回环对

双向 smoke 把两端都作为 FFI 对象驱动 —— 响应方服务带外 `ToolHandler`，拨号方服务反向调用，未注册工具被拒绝，处理器抛出的拒绝原样穿透：

```python
# 拨号方 FFI invoke_tool -> 响应方 FFI 带外 ToolHandler。
responder_sum = _SumToolHandler()
responder.register_tool_handler("tools.math.add", responder_sum)
sum_json = dialer.invoke_tool("tools.math.add", '{"a": 1, "b": 2}')
# sum_json == '{"sum": 3}'

# 响应方 FFI invoke_tool -> 经 RemoteAdapterFfi.register_tool_handler
# 注册的拨号侧处理器。
dialer_sum = _SumToolHandler()
dialer.register_tool_handler("tools.math.add", dialer_sum)
reverse_sum_json = responder.invoke_tool("tools.math.add", '{"a": 21, "b": 21}')
# reverse_sum_json == '{"sum": 42}'

# 已协商但未注册的工具 -> fail-closed op_unsupported。
try:
    dialer.invoke_tool("tools.echo.boom", "{}")
except spoke_connect.FfiError.Rejected as denied:
    assert denied.code == "CAPABILITY_PORT_MISSING"
    assert denied.wire_code == "op_unsupported"
```

处理器抛出的应用拒绝以相同形状原样穿透：

```python
class _ThrowingToolHandler:
    """Foreign-callback tool handler that always raises the given
    application reject."""

    def __init__(self, reject: spoke_connect.FfiError.Rejected) -> None:
        self._reject = reject

    def handle(self, arguments_json: str) -> str:
        raise self._reject


dialer.register_tool_handler(
    "tools.echo.boom",
    _ThrowingToolHandler(
        spoke_connect.FfiError.Rejected(
            "REVISION_CONFLICT", "foreign handler rejected", None, "op_unsupported"
        )
    ),
)
try:
    responder.invoke_tool("tools.echo.boom", "{}")
except spoke_connect.FfiError.Rejected as passed:
    assert passed.code == "REVISION_CONFLICT"
    assert passed.message == "foreign handler rejected"
    assert passed.wire_code == "op_unsupported"
```

### 工具路径错误

| 失败 | `FfiError` 行 |
|------|--------------|
| `invoke_tool` / `register_tool_handler` 上非 `tools.` 的 `capability_id`（两个包装对象皆然） | `Rejected`，`code: "INVALID_INPUT"`，零线上流量，违规 id 在 `message` 中 |
| `invoke_tool` 上畸形的 `arguments_json` | `Rejected`，`code: "INVALID_INPUT"`，零线上流量，解析错误在 `message` 中 |
| 分派拒绝 —— 工具未协商，或对等节点没有已注册处理器（对等节点应答 `op_unsupported` / `capability_missing`） | `Rejected`，`code: "CAPABILITY_PORT_MISSING"`，携带保留的对等节点 `wire_code` |
| 路由器没有有能力的对等节点 | `Rejected`，`code: "CAPABILITY_PORT_MISSING"`，`kind` = `wire_code` = `"no_capable_peer"`，能力 id 在 `message` 中 |
| 处理器抛出的 `Rejected` | 原样穿透 —— `code` / `message` 保留，`kind` / `wire_code` 保留在错误字段上 |
| 任何其它处理器结果（非契约 `Dial`、异常、panic） | `Rejected`，`code: "INTERNAL_ERROR"`，无 `kind` / `wire_code` —— 遏制，会话继续存活 |
| 单等待者调用超时 | `Rejected`，`code: "INTERNAL_ERROR"`，`kind: "timeout"` —— 仅该等待者，会话保持可用 |
| 调用期间会话关闭 / 传输 I/O | `Rejected`，`code: "INTERNAL_ERROR"`，`kind: "session_closed"` / `"transport"` |

### 线程与容量说明

处理器运行在 FFI 阻塞线程池（blocking pool）上：每次 `handle` 调用（与每个回调 `Transport` 方法一样）都经共享运行时的 `spawn_blocking` 池桥接，因此阻塞的带外调用绝不会独占 async worker。处理器**不得**同步回调 FFI 面 —— 重入（re-entrancy）会卡死会话；请把工作交给宿主自己的异步机制，然后从 `handle` 返回。

每个已建立的 FFI 会话在每个传输端固定占用一个阻塞线程池线程（接收循环阻塞在带外 `recv` 上）。按 tokio 默认的 512 个阻塞线程计，宿主进程在新增回调工作排队前大约可支撑 256 个全双工会话 —— 请据此为长期连接的规模设限。

## 7. 错误

每个 FFI 调用都经 `FfiError` 面结算 —— adapter 存在之前的拨号失败、invoke 路径的 `SpokeResult` 拒绝，以及回调 transport 自身的失败：

### `FfiError.Dial` —— 构造函数 / 拨号失败

`{ kind, message }`，在拨号于 adapter 存在之前失败时返回：

| `kind` | 何时 |
|--------|------|
| `config` | 本地种子或远端公钥不是恰好 32 字节、本地 manifest JSON 无效，或远端 `peer_id` 不在 allowlist 上（fail-closed） |
| `handshake` | 握手签名失败、nonce 单次使用违规、拨号绑定断言或 `ConnectSession` 快照校验失败（版本不匹配以 `protocol_version_mismatch` 呈现） |
| `protocol_version_mismatch` | hello 通告混合或未知的 `protocol_version` —— 版本门禁是握手校验第 1 步，先于签名校验，因此任何版本不匹配的 hello（无论签名是否有效）都以该种类失败 |
| `timeout` | 拨号截止时间已过（有界等待握手） |

### `FfiError.Rejected` —— invoke 路径的 `SpokeResult` 拒绝

`{ code, message, kind, wire_code }`，由已建立面上的 port 方法返回：

| 行 | 形状 |
|----|------|
| 应用拒绝 | `code` 原样保留（例如 `KNOWLEDGE_ENTRY_NOT_FOUND`） |
| 载荷 JSON 解析失败 | `INVALID_INPUT`，无 `kind` / `wire_code` |
| `INTERNAL_ERROR` 行 | `kind` ∈ {`transport`、`session_closed`、`timeout`、`panic`、`correlation_mismatch`、`sequence_exhausted`、`envelope_auth_missing`、`envelope_auth_invalid`、`envelope_auth_session_unbound`} |
| 分派拒绝 | `CAPABILITY_PORT_MISSING`，`wire_code` = `op_unsupported` / `capability_missing` |
| 未知线上码 | `INVALID_INPUT`，带 `wire_code` |
| 路由器终结拒绝 | `CAPABILITY_PORT_MISSING`，`wire_code` = `kind` = `no_capable_peer` |

`kind = "panic"` 是 panic 遏制行：导出 block-on-async 调用周围捕获的 panic 只让该等待者失败，绝不会跨 FFI 边界展开 —— message 携带原始 panic 载荷。`spawn_blocking` 池内的带外回调 panic 以传输 `Io` 失败呈现。

### `TransportError` —— 回调 transport 失败

| 变体 | 何时 |
|------|------|
| `Closed` | 连接关闭；挂起的 `recv` 快速失败 |
| `Io` | 传输级 I/O 失败，包括带外回调 panic 使阻塞 join 失败 |

## 各绑定符号对照表

| 面 | C# | Go | Kotlin | Python | Swift |
|---------|----|----|--------|--------|-------|
| 拨号并构造 | `ConnectRemoteAdapterFfi(...)` | `ConnectRemoteAdapterFfi(...)` | `connectRemoteAdapterFfi(...)` | `connect_remote_adapter_ffi(...)` | `connectRemoteAdapterFfi(...)` |
| Adapter 对象 | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` | `RemoteAdapterFfi` |
| 路由器构造 | `NewMultiPeerRouterFfi()` | `NewMultiPeerRouterFfi()` | `newMultiPeerRouterFfi()` | `new_multi_peer_router_ffi()` | `newMultiPeerRouterFfi()` |
| 路由器对象 | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` | `MultiPeerRouterFfi` |
| port 方法 | PascalCase（`GetKnowledgeEntry`） | PascalCase（`GetKnowledgeEntry`） | camelCase（`getKnowledgeEntry`） | snake_case（`get_knowledge_entry`） | camelCase（`getKnowledgeEntry`） |
| 工具调用 | `InvokeTool(...)` | `InvokeTool(...)` | `invokeTool(...)` | `invoke_tool(...)` | `invokeTool(capabilityId:argumentsJson:)` |
| 工具服务注册 | `RegisterToolHandler(...)` | `RegisterToolHandler(...)` | `registerToolHandler(...)` | `register_tool_handler(...)` | `registerToolHandler(capabilityId:handler:)` |
| 工具处理器回调 | `ToolHandler` | `ToolHandler` | `ToolHandler` | `ToolHandler` | `ToolHandler` |
| 响应方构造 | `SpokeConnectMethods.ConnectResponderFfi(...)` | `NewConnectResponderFfi(...)` | `connectResponderFfi(...)` | `connect_responder_ffi(...)` | `connectResponderFfi(...)` |
| 响应方对象 | `ConnectResponderFfi` | `ConnectResponderFfi` | `ConnectResponderFfi` | `ConnectResponderFfi` | `ConnectResponderFfi` |

每个绑定都携带相同的面：golden-parity smoke 断言字节级一致的会话核心行为，回环 smoke 从各宿主侧驱动回调 `Transport` + `RemoteAdapterFFI` 流程。

## 下一步

- [跨多个对等节点路由](/zh/how-to/multi-peer-routing) —— `MultiPeerRouterFFI` 背后的选择契约。
- [暴露并调用远程工具](/zh/how-to/connect-remote-tools) —— FFI 工具面所镜像的发现与反向调用契约。
- [从原生绑定连接](/zh/how-to/connect-native-bindings) —— 安装并认证每个绑定渠道。
- [从 TypeScript 客户端连接](/zh/how-to/connect-ts-client) —— 共享同一套会话核心规则的语言原生客户端。
- [connect 线上参考](/zh/reference/connect) —— 信封签名、身份绑定与会话核心规则。
