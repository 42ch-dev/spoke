---
title: 从 TypeScript 客户端连接
---

# 从 TypeScript 客户端连接（Connect from the TypeScript client）

**语言原生客户端**（`@42ch/spoke-connect`）用 TypeScript 实现 connect 线上契约与会话核心规则：`peer_id` 推导、Ed25519 握手签名、RFC 8785 JCS 规范化、每条消息一个 JSON 的 WebSocket 组帧，以及纯会话核心规则（allowlist、nonce、sequence、correlation、dispatch gate、capability token）。它配合平台 WebSocket 使用 —— 无需 Rust 运行时。

```bash
pnpm add @42ch/spoke-connect@X.Y.Z
```

入口：

- **`.`** —— 同构核心：身份、密码学、JCS 与会话核心。浏览器与 Node 均可用。
- **`./node`** —— Node 版 `connectClient`（依赖 `ws`），负责拨号 WebSocket 并完成握手。
- **`./noise`** —— 可选的 Noise XX mesh 传输子路径，用于直接的 libp2p-noise 互操作（见 [Noise 传输子路径](#noise-传输子路径)）。其依赖仅在导入该子路径时加载，默认的 `.` 与 `./node` 包体保持精简。

## 身份

```ts
import { derivePeerIdFromEd25519Pubkey, ed25519PubkeyFromPeerId, getPublicKeyEd25519 } from "@42ch/spoke-connect";

const publicKey = getPublicKeyEd25519(seed);            // 32-byte Ed25519 public key
const peerId = derivePeerIdFromEd25519Pubkey(publicKey); // 线上 peer_id（base58btc）
const roundTrip = ed25519PubkeyFromPeerId(peerId);       // 反向推导
```

该推导即协议身份绑定：protobuf `PublicKey` → 身份 multihash `0x00` → base58btc。与 Rust 参考实现及全部原生绑定的字节一致性由共享 golden vectors 锁定。

## 密码学与 JCS

```ts
import { base64UrlEncode, signEd25519, verifyEd25519, webcryptoEd25519Available } from "@42ch/spoke-connect";

const bytes = new TextEncoder().encode("...");
const signature = await signEd25519(seed, bytes);
const ok = await verifyEd25519(publicKey, bytes, signature);
```

Ed25519 在可用处使用 WebCrypto，同一代码路径下以 `@noble/ed25519` 回退（`webcryptoEd25519Available()` 报告当前路径）。签名以无填充 base64url 编码（[RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648)）。

`canonicalHelloBytes(peerId, nonce, host)` 生成签名握手对象 `{protocol_version, peer_id, nonce, host}` 的 RFC 8785 JCS 字节（[RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)）—— 缺失的可选成员不出现在规范化对象中。

## 签名握手与重放保护

```ts
import { generateNonce, signHelloEd25519, verifyHelloEd25519, NonceStore } from "@42ch/spoke-connect";

const nonce = generateNonce(); // 16 CSPRNG 字节，base64url
const hello = await signHelloEd25519(seed, nonce, manifest);

const store = new NonceStore();
store.checkAndRecord(remotePeerId, hello.nonce); // (peer_id, nonce) 对已接受过时返回 false
await verifyHelloEd25519(remotePubkey, remotePeerId, hello);
```

nonce 下限为 16 字符；`signHelloEd25519` 强制该下限（`invalid_nonce`）。`NonceStore` 只记录已接受的握手，因此被更早门禁拒绝的握手仍可重试。

## allowlist 与 dispatch

```ts
import { isAllowlisted, dispatchAllowed, requiredCapability, CAPABILITY_SPOKE_BASELINE } from "@42ch/spoke-connect";

isAllowlisted(["12D3KooW..."], peerId);         // fail-closed：空 allowlist 拒绝全部
dispatchAllowed("check", ["spoke-baseline"]);   // 核心 op 能力 ⊆ 协商能力
requiredCapability("check");                    // 核心 op 返回 "spoke-baseline"；产品 op 返回 null
```

dispatch gate 把核心 op 映射到所需能力（`upsert`、`promote`、`relate`、`check`、`assemble`、`project`、`compute`），未知 op 一律关闭（fail closed）。

## Capability token（能力令牌）

```ts
import { issueCapabilityToken, verifyCapabilityToken, TOKEN_VERSION, CLOCK_SKEW_SECONDS } from "@42ch/spoke-connect";

const proof = await issueCapabilityToken(issuerSeed, {
  iss: issuerPeerId,            // 由 issuerSeed 的公钥推导
  sub: subjectPeerId,           // 谁可以出示该令牌
  aud: verifierPeerId,          // 验证节点自己的 peer_id
  capabilities: ["spoke-baseline"],
  exp: Math.floor(Date.now() / 1000) + 60,
  iat: Math.floor(Date.now() / 1000),
});

const granted = await verifyCapabilityToken(
  proof,
  [issuerPeerId],               // 受信任签发方（fail-closed）
  thisPeerId,                   // 本节点 peer_id（aud 校验）
  sessionPeerId,                // 已认证的会话对端
  Math.floor(Date.now() / 1000),
);
// granted = 供 dispatch gate 使用的已校验能力列表
```

Capability token 是离线可校验、按能力授权的授权证明：受信任签发方以 Ed25519 对一组短声明（`iss` / `sub` / `aud` / `capabilities` / `exp`，可选 `iat` / `jti`）做 JCS 签名，证明经鉴权挑战/响应交换或逐 invoke 的 `auth` 携带。校验强制签发方信任、主体/受众绑定、过期与时钟偏差（`CLOCK_SKEW_SECONDS`）。

## 会话状态

```ts
import { Session, negotiatedCapabilities, OutboundSequence, InboundSequence, checkResponseCorrelation } from "@42ch/spoke-connect";

const session = new Session({
  session_id: "sess_1",
  initiator_peer_id: localPeerId,
  responder_peer_id: remotePeerId,
  negotiated_capabilities: negotiatedCapabilities(localCaps, remoteCaps),
});

session.allocateOutboundSequence(); // 0, 1, 2, … —— 超过 2^53−1 不回绕
```

每个会话、每个方向的 `sequence` 计数器从 0 开始；耗尽即关闭会话并开启新会话。响应回显 `session_id` / `sequence` / `request_id` —— `checkResponseCorrelation` 强制该匹配。`negotiatedCapabilities` 计算双方能力列表的协商子集（交集）。

## 使用 `connectClient` 端到端

Node 客户端完成完整流程 —— 拨号、签名握手交换、会话快照校验、带关联的 invoke：

```ts
import { derivePeerIdFromEd25519Pubkey } from "@42ch/spoke-connect";
import { connectClient } from "@42ch/spoke-connect/node";

const client = await connectClient({
  url: "ws://127.0.0.1:8080",
  identity: { seed },
  manifest: {
    schema_version: 1,
    host_id: "host_primary",
    roles: ["data-store"],
    capabilities: ["spoke-baseline"],
    namespaces: ["toy_world"],
    extensions: {},
  },
  remotePubkey,
  allowlist: [derivePeerIdFromEd25519Pubkey(remotePubkey)],
});

const response = await client.invoke("check", { scope: { scope_id: "book-harbor" } });
client.close();
```

当远端 `peer_id` 不在 allowlist 中时，客户端在拨号前即拒绝；每次握手与 invoke 的等待都以 `timeoutMs`（默认 5000）为上限。

## 浏览器 vs Node

核心导入（`@42ch/spoke-connect`）可在浏览器替换使用 —— Node 客户端及其 `ws` 依赖留在 `./node` 子路径。浏览器消费者只导入核心，并配合原生 WebSocket。

## Noise 传输子路径

需要直接的 libp2p-mesh 安全传输时，语言原生客户端提供可选的 Noise 子路径：

```ts
import { NoiseXX, NoiseTransport, createNoiseStaticKeypair } from "@42ch/spoke-connect/noise";
```

`@42ch/spoke-connect/noise` 是纯 TS Noise XX 栈 —— `Noise_XX_25519_ChaChaPoly_SHA256`（X25519 + ChaCha20-Poly1305 + HKDF-SHA256）—— 与 rust-libp2p Noise 参考实现线上兼容。静态密钥是真实的 X25519 密钥；握手第 2–3 飞携带 `NoiseHandshakePayload`，把静态密钥绑定到 SPOKE 对等节点的长期 Ed25519 身份（对 `"noise-libp2p-static-key:" || static_public` 的签名），与 rust-libp2p 对等节点的预期一致。

子路径自带其依赖（`@noble/ciphers`、`@noble/curves`）；从 `.` 或 `./node` 导入核心保持默认包体精简。SPOKE connect 握手与会话核心规则运行在 Noise 传输层之上。

## 对端互操作

TypeScript 客户端与 Rust 参考实现（crates.io 上的 `spoke-connect`）及每个原生绑定讲同一套会话核心规则 —— 共享契约见[connect 线上参考](/zh/reference/connect)。Rust 侧对端请 `cargo add spoke-connect`，并参考[crate README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md)中的两节点示例。

## 下一步

- [开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 每个辅助函数背后的概念，逐步讲解。
- [从原生绑定连接](/zh/how-to/connect-native-bindings) —— 从 C#、Kotlin、Swift、Go 或 Python 使用同一会话核心。
- [connect 线上参考](/zh/reference/connect) —— 信封字段表与身份绑定。
