---
title: connect 参考
---

# connect 参考（Connect reference）

connect 是面向跨进程 SPOKE 主机的可选**交互信封族**（`spoke-connect` 能力标志）：签名清单交换、会话上下文、远程 op 调用与可扩展鉴权。该族是增量的 —— 基线合规与基线 schema 保持不变。以下字段表溯源到 [`schemas/connect/`](https://github.com/42ch-dev/spoke/tree/main/schemas/connect) 中的已提交 schema。

## 六个信封

### ConnectHello —— 签名清单交换

必填：`protocol_version`、`peer_id`、`nonce`、`host`、`signature`、`extensions`。

| 字段 | 说明 |
|------|------|
| `protocol_version` | connect 协议版本（区别于数据 `schema_version`）；当前为版本 1 |
| `peer_id` | 发送方网络身份 —— 协议 v1：Ed25519 的 libp2p identity-spec PeerId 字符串（protobuf `PublicKey` 的 base58btc 身份 multihash）。对协议逻辑不透明；`noise-peerid` allowlist 的信任根 |
| `nonce` | 单次使用重放 nonce，绑定进签名对象 |
| `host` | 完整内嵌 `HostCapabilityManifest`（含 `host.extensions`）；属于签名对象的一部分 |
| `signature` | 对 JCS 规范化签名对象的原始签名字节，base64url（无填充）编码 |
| `extensions` | 产品字段袋；不在签名覆盖范围内 |

### ConnectSession —— 已建立的会话上下文

必填：`session_id`、`initiator_peer_id`、`responder_peer_id`、`opened_at`、`negotiated_capabilities`、`initial_sequence`、`extensions`。

| 字段 | 说明 |
|------|------|
| `session_id` | 不透明会话 id（建议 UUID；schema 不强制） |
| `initiator_peer_id` / `responder_peer_id` | 拨号的对端 / 接受的对端 |
| `opened_at` | 会话开启时间（UTC） |
| `negotiated_capabilities` | 双方 `capabilities[]` 的交集（或协商子集）；双方都声明时包含 `spoke-connect` |
| `initial_sequence` | 首次 invoke 请求使用的序列（协议版本 1 为 0） |
| `extensions` | 产品 namespace 袋 |

### ConnectInvokeRequest / ConnectInvokeResponse —— 远程 op 调用

`ConnectInvokeRequest` 必填：`session_id`、`sequence`、`request_id`、`op`、`payload`、`extensions`。

| 字段 | 说明 |
|------|------|
| `session_id` | 不透明会话 id |
| `sequence` | 本发送方按会话单调递增的出站序列；逻辑 u64，上限 2^53−1（JSON 安全） |
| `request_id` | 调用方生成的关联 id（建议 UUID） |
| `op` | 开放词汇。核心列表（记录在案，不强制）：`upsert`、`promote`、`relate`、`check`、`assemble`、`project`、`compute` |
| `payload` | 不透明 JSON —— 面向 SPOKE ops 时，必须是所命名 op 的完整既有 ops 请求信封 |
| `auth` | 可选会话中证明块；主要鉴权是握手。使用时形状按方法决定 |
| `extensions` | 产品 namespace 袋 |

`ConnectInvokeResponse` 是成功 `{ payload }` **或** `{ error }` —— 与 ops 线上相同的单一失败方言；失败复用共享 `ErrorEnvelope`。

### ConnectAuthChallenge / ConnectAuthResponse —— 可扩展鉴权

`ConnectAuthChallenge` 必填：`challenge_id`、`method`、`challenge`、`extensions`。`ConnectAuthResponse` 必填：`challenge_id`、`method`、`proof`、`extensions`。

| 字段 | 说明 |
|------|------|
| `challenge_id` | 关联 id，由响应回显 |
| `method` | 开放词汇。核心列表（记录在案，不强制）：`noise-peerid`、`capability-token`；保留名：`did` |
| `challenge` / `proof` | 不透明、按方法区分的材料；对 `capability-token`，`proof` 为 `{ v, claims, sig }`，`sig` 只覆盖 JCS(`claims`) |

## 身份

`peer_id` 是网络信任根：libp2p identity-spec PeerId（Ed25519，base58btc 身份 multihash）。`host_id` 是内嵌 `HostCapabilityManifest` 中的咨询性应用标签。两者保持独立 —— 一个主机可随时间呈现多个 peer id，而一个 peer id 恰好由一个 Ed25519 公钥推导。

## 签名握手（`spoke-connect-hello-jcs-v1`）

1. 连接双方交换已签名的 `ConnectHello`。
2. 签名对象恰好是 `{protocol_version, peer_id, nonce, host}` —— 顶层 `extensions` 与 `signature` 排除在外。
3. 对象经 RFC 8785 JCS 规范化（[RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)）。
4. 字节用 Ed25519 密钥对签名；原始签名以无填充 base64url 编码（[RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648)）。
5. 接收方仅在以下全部满足时接受：协议版本为 1、声称的 `peer_id` 等于已认证的远端对等节点、密钥能推导出该 peer id、对等节点在已配置 allowlist 中（空 allowlist 拒绝全部 —— fail-closed）、签名校验通过、且 `(peer_id, nonce)` 对是新的（单次使用，进程生命周期）。

## 排序与关联

每个会话、每个方向的单调 `sequence` 计数器从 0 开始；序列溢出会关闭会话并开启新会话。invoke 响应回显 `session_id` / `sequence` / `request_id` —— 任何不匹配都会使关联校验失败。接收方强制入站序列单调性，并以 `invalid_sequence` 线上信封应答重放或乱序序列。

## 鉴权方法

| 方法 | 工作原理 |
|------|----------|
| `noise-peerid` | 握手默认：allowlist 准入加签名握手，远端对等节点由传输层（noise）认证 |
| `capability-token` | 提权 / 会话中授权：受信任签发方以 Ed25519 对一组短声明（`iss` / `sub` / `aud` / `capabilities` / `exp`，可选 `iat` / `jti`）做 JCS 签名；证明经挑战/响应交换或逐 invoke 的 `auth` 携带。校验强制签发方信任、主体/受众绑定、过期与时钟偏差。受信任签发方列表为空时该方法被禁用（fail-closed） |

## 发现与显式对等连接

**显式对等连接（explicit peering）是生产路径**：主机配置监听地址，并经带外方式互相拨号（配置的地址或直接拨号）。connect 线上不携带任何发现字段 —— mDNS 只是 Rust 参考栈可选 `mdns` feature 提供的同 LAN 开发便利，发现的候选与显式拨号的对等节点一样，经同一 allowlist 与签名握手门禁准入。

## 传输

每条消息一个 JSON connect 信封，承载于有序、可靠、双向的字节流（TCP、WebSocket、yamux、libp2p request-response）。组帧分隔符、重试与载荷上限由传输 adapter 负责。

## 嵌入模型

| 嵌入方式 | 交付物 |
|----------|--------|
| **语言原生客户端（language-native client）** | 在宿主语言中实现的线上契约与会话核心规则（TypeScript `@42ch/spoke-connect` 客户端，WebSocket 传输） |
| **原生绑定（native bindings）** | 经 FFI 导出到宿主语言的共享会话核心（C# NuGet、Kotlin Maven、Swift SPM、Go modules、Python PyPI） |
| **Rust 参考实现（Rust reference）** | 已发布的 `spoke-connect` crate：会话核心参考、uniffi 绑定来源，以及 rust-libp2p 传输栈（[crate README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md)） |

会话核心规则 —— allowlist、`peer_id` 推导与反推、握手密码学、nonce、请求关联、sequence、capability-token 鉴权与 dispatch gate —— 在所有语言间共享，并由 golden vectors 锁定。薄客户端便利（`Session`、`negotiatedCapabilities`、`generateNonce`）在宿主运行时受益处提供。

## 相关页面

- [开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 端到端流程。
- [从 TypeScript 客户端连接](/zh/how-to/connect-ts-client) —— 语言原生客户端表面。
- [从原生绑定连接](/zh/how-to/connect-native-bindings) —— 带安装固定的 FFI 绑定。
- [协议参考](/zh/reference/protocol) —— `spoke-connect` 能力标志。
