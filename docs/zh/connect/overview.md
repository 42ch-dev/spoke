---
title: 连接总览
---

# 连接总览（Connect overview）

connect（连接层）是 SPOKE 主机之间的**交互信封族**（interaction envelope family）—— 可选加入（opt-in，`spoke-connect` 能力标志）：已签名清单交换、会话上下文、远程操作调用与可扩展鉴权。该信封族是叠加式的 —— 基线合规与基线 schema 保持不变。

本页为[英文原页](/connect/overview)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 六种信封

- **ConnectHello** —— 握手：已签名清单交换（经 `$ref` 内嵌 `HostCapabilityManifest`（主机能力清单））。
- **ConnectSession** —— 已建立的会话上下文快照（对等节点标识、协商能力、初始序列）。
- **ConnectInvokeRequest / ConnectInvokeResponse** —— 远程操作调用：将既有 ops 信封作为不透明 `payload` 包裹；失败复用共享 `ErrorEnvelope`。
- **ConnectAuthChallenge / ConnectAuthResponse** —— 方法可扩展鉴权（`method` 为开放字符串）。

## 设计规则

- **复用** —— hello 内嵌数据层清单；invoke 包裹 ops 信封；身份即既有不透明 `peer_id`（对等节点标识）。
- **身份** —— `peer_id`（信任根；协议 v1 中为 libp2p Ed25519 PeerId）与 `host_id`（主机标识，清单内的咨询性标签）相区别。
- **已签名握手** —— `spoke-connect-hello-jcs-v1`：对 `{protocol_version, peer_id, nonce, host}` 做 RFC 8785 JCS，Ed25519 签名，无填充 base64url。
- **排序** —— 每会话、每方向从 0 开始的单调 `sequence`；序列溢出时关闭当前会话并开启新会话；响应回显 `session_id` / `sequence` / `request_id`。
- **鉴权** —— `noise-peerid`（握手时的 allowlist 加已签名 hello）与 `capability-token`（能力令牌：离线验证、按能力范围的分级授权）。
- **发现** —— 显式 peering（配置地址 / 带外拨号）是规范生产路径；mDNS 仅为同局域网开发的非默认参考便利。

## 嵌入模型

- **语言原生客户端** —— 在宿主语言中实现线上与会话核心规则（见 [TypeScript 路线](/zh/connect/ts-route)）。
- **原生绑定** —— 经 FFI 将共享会话核心嵌入宿主语言（见 [原生绑定](/zh/connect/bindings)）。

## 传输

每条消息一个 JSON connect 信封，承载于有序、可靠、双向的字节流（TCP、WebSocket、yamux、libp2p request-response）。分帧分隔符、重试与 payload 上限由传输适配器负责。Rust 参考实现（`crates/spoke-connect`）提供 libp2p 传输与 uniffi 绑定面。

## 规范参考

- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) —— 信封字段表、身份绑定、JCS、会话核心状态机、鉴权模型、发现边界、传输分帧
- [schemas/connect/](https://github.com/42ch-dev/spoke/tree/main/schemas/connect) —— 已提交的 connect schema
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— connect 词汇（`peer_id`、capability token、Session）
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) —— `spoke-connect` 能力标志
