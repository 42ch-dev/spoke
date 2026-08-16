---
title: Connect 架构
---

# Connect 架构（Connect architecture）

**Connect** 是面向跨进程 SPOKE 主机的可选**交互信封族**（`spoke-connect` 能力标志）：签名清单交换、会话上下文、远程 op 调用与可扩展鉴权。它是增量的 —— 基线合规与基线 schema 保持不变，未声明 `spoke-connect` 的主机不受影响。

整个家族是一条完整的集成方旅程：安装 → 语言原生客户端会话 → 基于消费方 `Transport` 的 RemoteAdapter → 跨多个对等节点路由 → 原生绑定 → 回环冒烟测试。本页解释该旅程背后的概念；[教程](/zh/tutorials/first-connect-session)带着你走一遍，[how-to 指南](/zh/how-to/connect-remote-adapter)是配方，[线上参考](/zh/reference/connect)是字典。

## 三种嵌入面

| 面 | 交付物 | 何时选择 |
|----|--------|----------|
| **语言原生客户端（language-native client）** | 以宿主语言实现的线上契约与会话核心规则 —— TypeScript `@42ch/spoke-connect` 客户端，配合平台 WebSocket | 宿主无 Rust 运行时；浏览器或 Node 消费方 |
| **原生绑定（native bindings）** | 经 FFI 导出到宿主语言的共享会话核心（C# NuGet、Kotlin Maven、Swift SPM、Go modules、Python PyPI） | 有 FFI 故事的宿主语言，希望核心只实现一次、传输留在宿主 |
| **Rust 参考实现（Rust reference）** | 已发布的 `spoke-connect` crate：会话核心参考、绑定来源与 rust-libp2p 传输栈 | Rust 消费方，以及各处字节级一致性的参考 |

三个面共享同一套会话核心规则 —— `peer_id` 推导、握手密码学、allowlist、nonce、sequence、关联校验、dispatch gate —— 由 golden vectors 锁定。

## 会话生命周期

connect 会话经过四个状态：`Disconnected` → `Handshaking` → `Established` → `Closed`。

握手是一次签名 hello 交换。双方各对一个规范化对象签名 —— `{protocol_version, peer_id, nonce, host}`（发起方）或同样的对象加 `peer_nonce`（响应方）—— 使用各自的 Ed25519 对等节点身份。响应方的 `peer_nonce` 回显发起方的 nonce，从而绑定拨号：捕获的响应方 hello 无法重放进新的拨号。准入在 allowlist 上 fail-closed，每个已接受的 `(peer_id, nonce)` 对单次使用。

双方 hello 都被接受后，会话快照（`ConnectSession`）记录会话 id、绑定的对等节点标识、协商能力与起始序列。从 `Established` 起，每个对等节点维护自己按方向从 0 开始的 `sequence` 计数器，每次 invoke 携带调用方生成的 `request_id`；响应必须回显 `session_id`、`sequence` 与 `request_id`，否则关联校验失败。序列耗尽会关闭会话而不是回绕 —— 新会话从干净的计数器开始，而不是复用序列空间。

## 信封认证

信封真实性是**传输层之上的协议级属性**：每条影响信任的 post-hello 信封 —— 会话快照、invoke 请求与 invoke 响应 —— 都携带对 RFC 8785 JCS 规范化签名对象的 Ed25519 签名，在会话核心内部校验。其构造与 hello 相同，扩展为三个算法 id（`spoke-connect-session-jcs-v1`、`spoke-connect-invoke-request-jcs-v1`、`spoke-connect-invoke-response-jcs-v1`）。协议版本 **2** 使这些签名在 post-hello 线上成为必填；hello 交换本身保持在版本 1。

这就是 adapter 能在任何有序、可靠的载体上工作（TCP、WebSocket、yamux 或 Noise）而不必信任传输层真实性的原因：信封本身已认证，且提供已认证对等节点身份的传输不会放宽规则。签名对象覆盖影响信任的字段；`extensions` 留在签名之外，因此永远不会影响授权。混合版本对等节点 fail-closed：v2 对等节点拒绝与 v1 对等节点建立会话，而不是接受未认证信封，且不存在兼容垫片（compatibility shim）。

## 能力路由

**RemoteAdapter（远程适配器）**通过在已建立会话上把每个 port 调用作为保留的 `port.*` op 代理，实现异步 `BaselinePorts` adapter 契约 —— 远端主机的 port 面看起来就在本地。**多对等节点路由器（multi-peer router）**在同一个 `BaselinePorts` 面之后组合 N 个已注册 adapter，因此 `orchestrateUpsert(router, req)` 能触达有能力的对等节点而无需指名。

路由器的选择是已注册对等节点集与请求的纯函数：对等节点声明能力（op 的必需能力）、namespaces 与 `authority.scope_key` 上的硬门禁；对 op 首选角色的软偏好；以及确定性的最小 `peer_id` 决胜。当没有已注册对等节点通过硬门禁时，调用以 `no_capable_peer` 拒绝 —— 消费方注册一个满足条件的对等节点，并用新的 `request_id` 重新调用。重试由消费方负责：调用可能在传输失败前已被应用，因此由消费方决定重新运行该操作是否安全。

能力有两个来源：会话的 `negotiated_capabilities`（双方列表的协商子集）与 capability token（能力令牌，来自受信任签发方的短期、按能力授权的授权证明）。令牌授权为其覆盖的 ops 授权成员资格，但并不取代协商集 —— 令牌门禁生效时，两者都必须允许该 op。

## 双向能力流

一条会话在两个方向承载能力流量，两个方向的形状不同：

| 方向 | 提供方 | 消费方 | 面 |
|------|--------|--------|-----|
| **ports** | 主机提供其本地 `BaselinePorts` | 拨号方以可即插即用的异步面消费 | 经 D4 目录的保留 `port.*` op |
| **工具（tools）** | 拨号方为其声明的工具注册处理器 | 主机从已认证清单中发现它们并在编排中途反向调用 | `tools.*` op，其 op 字符串就是工具能力 id |

port 方向是主机到客户端的消费：主机的 `connectResponder` 针对注入的 `BaselinePorts` 服务 `port.*` invoke，拨号方的 `RemoteAdapter`（远程适配器）把每个 port 方法代理为一次 invoke，因此远端 port 面看起来就在本地。工具方向是客户端到主机的提供：拨号方的清单声明 `tools[]`，拨号方在其 `RemoteAdapter` 上注册处理器，主机从已认证清单中列出这些工具，并以响应方的 `invokeTool` 面调用它们。

两个方向经同一机制协商 —— 能力字符串必须在 `negotiated_capabilities`（协商能力）中，其 op 才能被分派。因此工具与任何其它能力一样被协商：双方都列出 `tools.toy_world.roll_dice`，双方取交集，`tools.toy_world.roll_dice` invoke 就能在会话任一侧分派。工具发现是已认证会话的属性：主机读取的是它在握手时已校验的拨号方清单，因此不存在独立的通告往返。

拒绝路径是共享的：未协商的 op，或没有注册处理器的工具，以线上码 `op_unsupported` 应答，在调用方映射为 `CAPABILITY_PORT_MISSING` 拒绝 —— 编排观察到拒绝而非静默成功。完整配方见[暴露并调用远程工具](/zh/how-to/connect-remote-tools)，字段表与分派规则见[线上参考](/zh/reference/connect#工具-反向调用)。

## 传输层在哪里

传输是消费方实现的接缝。connect 软件包定义一个消息导向的 `Transport`（传输接口）—— 每次 `send` / `recv` 调用一个 connect 信封，`recv` 阻塞直到信封到达或连接关闭，`close` 幂等 —— 并附带一个内存回环对（loopback，测试用途）供测试。WebSocket 与其它载体是同样的三个方法的消费方侧实现；字节流载体应用长度前缀（或等价方式）定界。回环对仅供测试 —— 经它拨号的冒烟测试是验证流程，不是生产载体。

## 相关页面

- [开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 逐步的学习路径。
- [通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter) —— 经消费方 `Transport` 拨号远端对等节点。
- [跨多个对等节点路由](/zh/how-to/multi-peer-routing) —— 一个路由器管理 N 个已注册 adapter。
- [暴露并调用远程工具](/zh/how-to/connect-remote-tools) —— 双向能力流中的工具方向，端到端。
- [从原生绑定连接](/zh/how-to/connect-native-bindings) —— 经 FFI 的共享会话核心。
- [connect 线上参考](/zh/reference/connect) —— 信封字段表与 v2 信封认证规则。
