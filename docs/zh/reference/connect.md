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
| `protocol_version` | 消费方在 `ConnectHello` 中设置的 `protocol_version` 值是 **1**（hello 交换未升版；hello 签名字段集与 `spoke-connect-hello-jcs-v1` 算法不变）。协议版本 **2 是当前的**规范性 connect 协议版本：它在 **post-hello** 线上增加必填的逐信封签名，由 RemoteAdapter 内部强制执行，消费方从不把它作为字段值设置（见[信封认证（protocol_version 2）](#信封认证-protocol-version-2)）。绑定的 `protocolVersion()` 报告的是 hello 版本（1）。 |
| `peer_id` | 发送方网络身份 —— 协议 v1：Ed25519 的 libp2p identity-spec PeerId 字符串（protobuf `PublicKey` 的 base58btc 身份 multihash）。对协议逻辑不透明；`noise-peerid` allowlist 的信任根 |
| `nonce` | 单次使用重放 nonce，绑定进签名对象 |
| `peer_nonce` | 仅响应方使用的拨号绑定：发起方的 nonce，由响应方回显并绑定进其签名对象。发起方 hello 中缺省；发起方拒绝 `peer_nonce` 与其自身 nonce 不一致的响应方 hello |
| `host` | 完整内嵌 `HostCapabilityManifest`（含 `host.extensions`）；属于签名对象的一部分 |
| `signature` | 对 JCS 规范化签名对象的原始签名字节，base64url（无填充）编码 |
| `extensions` | 产品字段袋；不在签名覆盖范围内 |

### ConnectSession —— 已建立的会话上下文

必填：`session_id`、`initiator_peer_id`、`responder_peer_id`、`opened_at`、`negotiated_capabilities`、`initial_sequence`、`extensions`（protocol_version 2 下另有 `signature`）。

| 字段 | 说明 |
|------|------|
| `session_id` | 不透明会话 id（建议 UUID；schema 不强制） |
| `initiator_peer_id` / `responder_peer_id` | 拨号的对端 / 接受的对端 |
| `opened_at` | 会话开启时间（UTC） |
| `negotiated_capabilities` | 双方 `capabilities[]` 的交集（或协商子集）；双方都声明时包含 `spoke-connect` |
| `initial_sequence` | 首次 invoke 请求使用的序列 —— 协议版本 1 与 2 均为常量 0 |
| `signature` | 仅 v2、必填、minLength 86 maxLength 86 —— 对 JCS 规范化签名对象的 64 字节 Ed25519 签名的 base64url（无填充）编码（`spoke-connect-session-jcs-v1`）；见[信封认证（protocol_version 2）](#信封认证-protocol-version-2) |
| `extensions` | 产品 namespace 袋 |

### ConnectInvokeRequest / ConnectInvokeResponse —— 远程 op 调用

`ConnectInvokeRequest` 必填：`session_id`、`sequence`、`request_id`、`op`、`payload`、`extensions`（protocol_version 2 下另有 `signature`）。

| 字段 | 说明 |
|------|------|
| `session_id` | 不透明会话 id |
| `sequence` | 本发送方按会话单调递增的出站序列；逻辑 u64，上限 2^53−1（JSON 安全） |
| `request_id` | 调用方生成的关联 id（建议 UUID） |
| `op` | 开放词汇。核心列表（记录在案，不强制）：`upsert`、`promote`、`relate`、`check`、`assemble`、`project`、`compute`；保留 `port.*` 前缀用于 RemoteAdapter port 方法（见[Port-method ops（RemoteAdapter）](#port-method-ops-remoteadapter)） |
| `payload` | 不透明 JSON —— 面向 SPOKE ops 时，必须是所命名 op 的完整既有 ops 请求信封 |
| `auth` | 可选会话中证明块；主要鉴权是握手。使用时形状按方法决定。在 protocol_version 2 线上存在时，`auth` 包含在 JCS 签名对象中 |
| `signature` | 仅 v2、必填、minLength 86 maxLength 86 —— 对 JCS 规范化签名对象的 64 字节 Ed25519 签名的 base64url（无填充）编码（`spoke-connect-invoke-request-jcs-v1`）；见[信封认证（protocol_version 2）](#信封认证-protocol-version-2) |
| `extensions` | 产品 namespace 袋 |

`ConnectInvokeResponse` 是成功 `{ payload }` **或** `{ error }` —— 与 ops 线上相同的单一失败方言；失败复用共享 `ErrorEnvelope`。两个分支在 protocol_version 2 下都增加必填 `signature`：

| 分支 | v2 `signature` |
|------|----------------|
| 成功 `{ session_id, sequence, request_id, payload, extensions }` | 必填，minLength 86 maxLength 86 —— 对 `{session_id, sequence, request_id, payload}` 的 `spoke-connect-invoke-response-jcs-v1` |
| 错误 `{ session_id, sequence, request_id, error, extensions }` | 必填，minLength 86 maxLength 86 —— 对 `{session_id, sequence, request_id, error}` 的 `spoke-connect-invoke-response-jcs-v1` |

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
2. **发起方** hello 的签名对象为 `{protocol_version, peer_id, nonce, host}`（4 个字段，`peer_nonce` 缺省）；**响应方** hello 的签名对象为 `{protocol_version, peer_id, nonce, host, peer_nonce}`（5 个字段，`peer_nonce` = 发起方的 nonce，即拨号绑定）。顶层 `extensions` 与 `signature` 排除在外。
3. 对象经 RFC 8785 JCS 规范化（[RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)）。
4. 字节用 Ed25519 密钥对签名；原始签名以无填充 base64url 编码（[RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648)）。
5. 接收方仅在以下全部满足时接受：协议版本为 1、声称的 `peer_id` 等于已认证的远端对等节点、密钥能推导出该 peer id、对等节点在已配置 allowlist 中（空 allowlist 拒绝全部 —— fail-closed）、签名按角色感知的字段集校验通过、且 `(peer_id, nonce)` 对是新的（单次使用，进程生命周期）。
6. 拨号绑定：发起方额外要求响应方签名的 `peer_nonce` 等于其自身 nonce —— 捕获的响应方 hello 无法重放进新的拨号（例如客户端重启导致内存 nonce 存储重置之后）。

## 信封认证（protocol_version 2）

协议版本 **2** 在协议层对每条 post-hello 影响信任的信封 —— `ConnectSession`、`ConnectInvokeRequest`、`ConnectInvokeResponse` —— 做认证，使用逐信封 JCS + Ed25519 签名字段集，构造与 `spoke-connect-hello-jcs-v1` 相同，扩展为三个新算法 id。接收方在协议层校验信封真实性，独立于传输层的 TLS 或 Noise；传输层提供的已认证对等节点身份不会放宽规则。

### 算法 id

| 算法 id | 信封 |
|---------|------|
| `spoke-connect-hello-jcs-v1` | `ConnectHello`（不变） |
| `spoke-connect-session-jcs-v1` | `ConnectSession` |
| `spoke-connect-invoke-request-jcs-v1` | `ConnectInvokeRequest` |
| `spoke-connect-invoke-response-jcs-v1` | `ConnectInvokeResponse` |

四个算法共享同一构造：RFC 8785 JCS → UTF-8 字节 → Ed25519 签名/校验 → 无填充 base64url。签名密钥是发送方的对等节点身份 Ed25519 私钥；校验使用推导出已认证握手 `peer_id` 的公钥。

### 已签名字段集

每个签名对象都是线上信封的严格子集（精确键，无其它）。`extensions` 与 `signature` 排除在外 —— `extensions` 不在签名覆盖范围内，不参与信任决策：

- **`ConnectSession`**：`{session_id, initiator_peer_id, responder_peer_id, opened_at, negotiated_capabilities, initial_sequence}`
- **`ConnectInvokeRequest`**：`{session_id, sequence, request_id, op, payload}`，且当 `auth` 在线上存在时**另加** `auth`
- **`ConnectInvokeResponse`** 成功分支：`{session_id, sequence, request_id, payload}`
- **`ConnectInvokeResponse`** 错误分支：`{session_id, sequence, request_id, error}`

两个响应分支分别对其字段集签名；`signature` 字段必须是 64 个原始签名字节的规范 base64url（无填充）编码。

### 校验规则

对每条 v2 post-hello 信封，接收方：（1）检查 `signature` 存在；（2）执行规范编码往返检查；（3）按锁定的字段集构造签名对象；（4）用 RFC 8785 JCS 规范化；（5）用对等节点的握手 Ed25519 公钥校验；（6）断言会话绑定（`session_id` 绑定到已建立会话，对等节点标识与已认证握手一致）。任一失败都拒绝该信封。签名绑定到会话：未绑定到已建立会话的 `session_id` 被拒绝，从一个会话捕获的信封重放进另一个会话会在另一个会话的握手密钥下校验失败。

### 版本策略

| 方向 | 行为 |
|------|------|
| v2 对等节点 ↔ v1 对等节点 | v2 侧在已校验握手中看到 `protocol_version: 1` 并拒绝建立 —— v1 侧无法产生带签名的会话/invoke 信封。拨号 fail-closed |
| 双方 v2 | 会话在 v2 规则下建立；所有 post-hello 信封携带必填 `signature` |
| 双方 v1 | 仅传统 v1 互操作 |
| 未知版本（> 2） | 通告未知版本的握手在版本门禁 fail-closed（版本检查是握手校验第 1 步，先于签名校验），按混合版本拨号处理 |

hello 签名字段集（4 字段发起方 / 5 字段响应方）不变；拨号绑定 `peer_nonce` 规则保留。

### 错误映射

信封认证失败使用共享 `ErrorEnvelope` 词汇：`auth_failed` 覆盖缺失、无效、非规范或字段集漂移的签名与会话绑定不匹配。在 RemoteAdapter 面上，这些以 `SpokeResult` 拒绝呈现 —— `INTERNAL_ERROR`，`details.kind` ∈ {`envelope_auth_missing`、`envelope_auth_invalid`、`envelope_auth_session_unbound`} —— 而混合或未知版本 hello 以专用种类使拨号失败：`RemoteAdapterError::ProtocolVersionMismatch`（Rust）/ `CoreError`（`code: "protocol_version_mismatch"`，TS），经 FFI 以 `FfiError.Dial`（`kind: "protocol_version_mismatch"`）呈现；无 adapter 实例。版本门禁是握手校验第 1 步 —— 先于签名校验 —— 因此任何版本不匹配的 hello（无论签名是否有效）都以该专用种类失败。

### 强制

RemoteAdapter（TypeScript `./remote` 子路径、Rust `remote-adapter` feature）与 connect 客户端在它们发射或接受的每条 post-hello 信封上内部强制 v2 逐信封认证，无需任何配置：拨号在建立时校验签名的 `ConnectSession` 快照，每个出站 invoke 请求都签名，每条关联响应在关联回显检查后校验。hello 交换保持在协议版本 1。`ConnectAuthChallenge` / `ConnectAuthResponse` 携带方法特定的证明，本身已绑定签名；它们不在 v2 逐信封签名范围内。

## 排序与关联

每个会话、每个方向的单调 `sequence` 计数器从 0 开始；序列溢出会关闭会话并开启新会话。invoke 响应回显 `session_id` / `sequence` / `request_id` —— 任何不匹配都会使关联校验失败。接收方强制入站序列单调性，并以 `invalid_sequence` 线上信封应答重放或乱序序列。

## 会话核心状态机

会话核心为每个本地节点、每个会话跟踪一个逻辑状态：

| 状态 | 含义 |
|------|------|
| `Disconnected` | 无传输会话；本会话无出站序列 |
| `Handshaking` | 传输已建立；hello 在途；invoke 尚未被授权 |
| `Established` | 双方 hello 均被接受；`session_id` 已分配；出站计数器 = 0；入站期望 = 0 |
| `Closed` | 会话不可用（序列耗尽、传输丢失、认证失败、本地关闭）；开启新会话 —— 序列不回绕 |

| 转换 | 触发 | 守卫 / 效果 |
|------|------|-------------|
| `Disconnected` → `Handshaking` | 传输连接 / 接受 | —— |
| `Handshaking` → `Established` | 本地接受远端 hello 且远端接受本地 hello | allowlist + 签名 + nonce 单次使用；拨号绑定（响应方签名的 `peer_nonce` = 发起方 nonce）；会话对等节点标识绑定到已认证 hello 的 `peer_id`；`negotiated_capabilities` = 协商子集；出站计数器 = 0 且入站期望 = 0 |
| `Handshaking` → `Closed` | 任一 hello 门禁失败 | 被拒绝 hello 的 nonce 不记录 |
| `Established` → `Established` | 出站 invoke | 原子分配 `sequence = last + 1`（从 0 开始）；附加新 `request_id`；发送 |
| `Established` → `Established` | 入站 invoke | 仅当 `sequence == next_expected_inbound`（从 0 开始）时接受，然后前进；否则以线上错误拒绝且无处理方副作用 |
| `Established` → `Established` | 入站响应 | 仅当回显某挂起请求的 `session_id`、`sequence` 与 `request_id` 时接受；否则关联失败 |
| `Established` → `Closed` | 下一个出站序列将超过 2^53−1、传输丢失或本地关闭 | 不回绕 |

在 v2 线上，序列/关联检查先运行，但在信封认证校验通过前不推进会话状态（见[信封认证（protocol_version 2）](#信封认证-protocol-version-2)）。

## 鉴权方法

| 方法 | 工作原理 |
|------|----------|
| `noise-peerid` | 握手默认：allowlist 准入加签名握手，远端对等节点由传输层（noise）认证 |
| `capability-token` | 提权 / 会话中授权：受信任签发方以 Ed25519 对一组短声明（`iss` / `sub` / `aud` / `capabilities` / `exp`，可选 `iat` / `jti`）做 JCS 签名；证明经挑战/响应交换或逐 invoke 的 `auth` 携带。校验强制签发方信任、主体/受众绑定、过期与时钟偏差。受信任签发方列表为空时该方法被禁用（fail-closed） |

## 能力词汇（Capability vocabulary）

每个操作映射到会话 `negotiated_capabilities` 上它所需的能力（dispatch gate 评估协商集，而非仅远端 manifest）：

| 操作 | 所需能力 |
|------|----------|
| `upsert`、`promote`、`relate`、`check`、`assemble`（以及 `port.*` 基线操作） | `spoke-baseline` |
| `project`、`compute`（以及 `port.computable.*`） | `l2-computable` |
| 产品自定义操作 | 产品文档化的能力 |

能力令牌授权为其 `capabilities[]` 覆盖的 ops 授权会话成员资格，但并不取代 `negotiated_capabilities` —— 令牌门禁生效时，令牌授权与协商集都必须允许该 op。

## Port-method ops（RemoteAdapter）

RemoteAdapter 把每个 `BaselinePorts` 方法代理为一个 connect invoke，携带保留的 `port.*` 产品 op 与不透明的 snake_case 载荷：

| 方法 | `op` | 请求 `payload` | 成功 `payload` |
|------|------|----------------|----------------|
| `getKnowledgeEntry` | `port.knowledge.get` | `{ "entry_id": string }` | `KnowledgeEntry` |
| `putKnowledgeEntry` | `port.knowledge.put` | `{ "entry": KnowledgeEntry, "expected_base_revision": number \| null }` | `KnowledgeEntry` |
| `getRelation` | `port.relation.get` | `{ "relation_id": string }` | `Relation` |
| `putRelation` | `port.relation.put` | `{ "relation": Relation, "expected_base_revision": number \| null }` | `Relation` |
| `listKnowledgeEntries` | `port.scope.list_knowledge_entries` | `{ "scope": Scope }` | `KnowledgeEntry[]` |
| `listTimelineEvents` | `port.scope.list_timeline_events` | `{ "scope": Scope }` | `TimelineEvent[]` |
| `putFindings` | `port.finding.put` | `{ "findings": Finding[] }` | `Finding[]` |
| `listRules` | `port.rule.list` | `{ "rule_refs": string[] }` | `Rule[]` |
| `listPeerHostCapabilityManifests` | `port.host.list_peer_manifests` | `{}` | `HostCapabilityManifest[]` |
| `getHostCapabilityManifest` | *（无 —— 会话缓存）* | —— | 建立时缓存的远端握手 `host`；无往返 |

可选族保留 `port.computable.*`（`project` / `compute`、`l2-computable`）与 `port.fork.*`（`listForkTimelineEvents`、`l5-fork`）供未来产品使用；基线 adapter 交付上表。

## 发现与显式对等连接

**显式对等连接（explicit peering）是生产路径**：主机配置监听地址，并经带外方式互相拨号（配置的地址或直接拨号）。connect 线上不携带任何发现字段 —— 发现属传输侧职责，会话准入仍完全由 allowlist 与签名握手门禁把关。

## 传输

每条消息一个 JSON connect 信封，承载于有序、可靠、双向的字节流（TCP、WebSocket、yamux、libp2p request-response）。组帧分隔符、重试与载荷上限由传输 adapter 负责。

## 嵌入模型

| 嵌入方式 | 交付物 |
|----------|--------|
| **语言原生客户端（language-native client）** | 在宿主语言中实现的线上契约与会话核心规则（TypeScript `@42ch/spoke-connect` 客户端，WebSocket 传输） |
| **原生绑定（native bindings）** | 经 FFI 导出到宿主语言的共享会话核心（C# NuGet、Kotlin Maven、Swift SPM、Go modules、Python PyPI） |
| **Rust 参考实现（Rust reference）** | 已发布的 `spoke-connect` crate：会话核心参考、uniffi 绑定来源，以及 rust-libp2p 传输栈（[crate README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md)） |

会话核心规则 —— allowlist、`peer_id` 推导与反推、握手密码学、nonce、请求关联、sequence、capability-token 鉴权与 dispatch gate —— 在所有语言间共享，并由 golden vectors 锁定。薄客户端便利（`Session`、`negotiatedCapabilities`、`generateNonce`）在宿主运行时受益处提供。

## 错误词汇

| 条目 | 出现位置 | 含义 |
|------|----------|------|
| `auth_failed` | `ErrorEnvelope.code` | 令牌认证失败（需要时缺失或无效令牌；签名 / 签发方 / 受众 / 主体 / 过期 / 畸形证明）以及全部信封认证失败（缺失、无效或非规范签名；字段集漂移；会话绑定不匹配） |
| `invalid_sequence` | `ErrorEnvelope.code` | 重放或乱序的入站序列 |
| `op_unsupported` | `ErrorEnvelope.code` | 未知 `op`，或令牌有效但缺乏所请求 op 的能力 |
| `capability_missing` | `ErrorEnvelope.code` | op 的必需能力不在有效授权中 |
| `no_capable_peer` | 路由器拒绝 `details.wire_code` / `details.kind` | 没有已注册对等节点通过硬选择门禁时的终结路由器拒绝（`CAPABILITY_PORT_MISSING`）；注册满足条件的对等节点并重新调用 |
| `envelope_auth_missing` / `envelope_auth_invalid` / `envelope_auth_session_unbound` | RemoteAdapter 拒绝 `details.kind` | `INTERNAL_ERROR` 拒绝上的信封认证拒绝种类（仅该等待者；会话状态不变） |
| `handshake` | 拨号失败 `details.kind`（`FfiError.Dial`） | 握手签名 / 身份 / nonce 校验失败（版本不匹配以 `protocol_version_mismatch` 呈现）；拨号面为 {`config`、`handshake`、`timeout`、`protocol_version_mismatch`}，无 adapter 实例 |
| `protocol_version_mismatch` | 拨号失败 `details.kind`（`FfiError.Dial`） | 通告混合或未知协议版本的 hello 以专用种类使拨号失败 —— `RemoteAdapterError::ProtocolVersionMismatch` / `CoreError("protocol_version_mismatch")`；无 adapter 实例 |
| `transport` / `session_closed` / `timeout` / `panic` / `correlation_mismatch` / `sequence_exhausted` | RemoteAdapter 拒绝 `details.kind` | 传输 I/O、会话丢失、invoke 超时、panic 遏制、关联不匹配与序列耗尽的 `INTERNAL_ERROR` 拒绝种类 |

## 相关页面

- [开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 端到端流程。
- [从 TypeScript 客户端连接](/zh/how-to/connect-ts-client) —— 语言原生客户端面。
- [通过 Transport 使用 RemoteAdapter](/zh/how-to/connect-remote-adapter) —— 经消费方 `Transport` 拨号远端对等节点并调用其 `BaselinePorts` 面。
- [跨多个对等节点路由](/zh/how-to/multi-peer-routing) —— 管理 N 个已注册 adapter 的路由器配方。
- [从原生绑定连接](/zh/how-to/connect-native-bindings) —— 带安装固定的 FFI 绑定。
- [Connect 架构](/zh/explanation/connect) —— 会话生命周期、信封认证与能力路由。
- [协议参考](/zh/reference/protocol) —— `spoke-connect` 能力标志。
