---
title: TypeScript 连接路线
---

# TypeScript 连接路线（TypeScript connect route）

connect 路径 A 的 TypeScript 路线是**纯 TS 最小实现（pure-TS-minimal）**：信封规则与身份计算用纯 JS 原语在 TypeScript 中实现（WebCrypto 可用时使用，`@noble/ed25519` 回退）—— js-libp2p 与 WASM 是下方的回退路线。

本页为[英文原页](/connect/ts-route)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 路线形态

- **传输** —— WebSocket 作为有序可靠流，每条消息一个 JSON connect 信封。
- **密码学** —— 可用时用 WebCrypto `Ed25519`，`@noble/ed25519` 回退（相同的 seed/sign 语义）。
- **规范化** —— 对已签名 hello 字段做 RFC 8785 JCS。
- **`peer_id`（对等节点标识）** —— 仅规范公式：protobuf `PublicKey` → 身份 multihash `0x00` → base58btc（无 multibase 前缀、无 CIDv1）。

## 会话核心能力对等（session-core parity）

TypeScript 客户端（`packages/spoke-connect-ts`）与 Rust 参考实现（`crates/spoke-connect`）的**会话核心保持能力对等**：双方共享的纯会话逻辑 —— allowlist、`peer_id`（派生与反向，用于令牌验证）、hello 密码学、nonce、请求关联、序列、capability-token（能力令牌）鉴权，以及 dispatch gate / 产品操作能力映射（含经 `tokenAuthorizesOp` 的令牌授予成员资格）—— 实现同一套规则，并以共享 golden vectors 与 round-trip 对等测试验证。传输保持刻意不对称（TS 侧 WebSocket、Rust 侧 libp2p 栈），不属于对等契约范围。

## 回退路线

- **js-libp2p** —— 当产品必须加入共享 libp2p mesh 并与 Rust 参考栈说 Noise/yamux 时的 mesh 回退。
- **WASM（Rust 核心 + JS 传输）** —— 纯 TS 密码学 / JCS 出现缺口时的延后回退路线。

## 证据

JavaScript 中的身份字节可复现性已验证：`tooling/connect-identity-proof/proof.mjs`（零 npm 依赖）匹配 Rust golden vectors 的 `peer_id`、JCS 字节、Ed25519 签名与 base64url 编码 —— 全部检查在 Node 24 WebCrypto 上 PASS。

## 实现范围

本路线覆盖纯辅助（`derivePeerId`、规范 hello 字节 / JCS、`signHello` / `verifyHello`、base64url）、一个 WebSocket 传输适配器，以及一个最小会话核心移植（协议版本检查、allowlist、nonce 一次性使用、每方向序列、`request_id` 关联、dispatch gate）。Swarm 功能、DHT 发现与已发布的 npm connect 软件包在一期范围之外。

## 规范参考

- [spoke-connect-ts-route.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect-ts-route.md) —— 完整评估、论证、推翻检查、身份证明证据
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) —— 规范线上 / 身份 / 分帧（本路线保持信封不变）
- [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md) —— TS 面的发布与打包策略
- [tooling/connect-identity-proof/](https://github.com/42ch-dev/spoke/tree/main/tooling/connect-identity-proof) —— 本地 JS 可复现性证明
