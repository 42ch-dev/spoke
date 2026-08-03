---
title: 原生绑定
---

# 原生绑定（Native bindings，Path B）

路径 B 将**共享会话核心**经 FFI 嵌入宿主语言：纯同步的会话规则（`peer_id`（对等节点标识）派生、hello 签名/验证、allowlist、nonce 一次性使用、序列分配、关联、dispatch gate）全部位于一个核心中，传输留在各宿主语言。参考实现是 `spoke-connect` crate 的 **Binding facade**。

本页为[英文原页](/connect/bindings)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## Binding facade

crate 的 `src/core/` 是纯同步、语言可移植层 —— libp2p、tokio 与 I/O 位于传输层，传输层在边界处转换 `libp2p::PeerId` ↔ `String`。facade 决策记录在 crate README 的 [Binding facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) 一节，含同步/异步边界与导出面。

## C# NuGet（`42ch.Spoke.Connect`）

集成方通过 **GitHub Packages** 消费会话核心：

```xml
<!-- nuget.config（每个解决方案配置一次） -->
<packageSources>
  <add key="github-42ch" value="https://nuget.pkg.github.com/42ch-dev/index.json" />
</packageSources>
```

```xml
<PackageReference Include="42ch.Spoke.Connect" Version="0.7.1" />
```

原生 `spoke_connect` / `libspoke_connect` 位于 NuGet `runtimes/<rid>/native/`（`win-x64`、`linux-x64`、`osx-arm64`）。包版本与 spoke 锁步 SemVer / 标签 `vX.Y.Z` 对齐。

## Swift 同步核心绑定

**Swift 同步核心骨架**已通过 uniffi 在可选 `ffi` 功能（`cdylib`）下落地：导出 `derivePeerIdFromEd25519Pubkey`、`signHelloEd25519` / `verifyHelloEd25519`、`isAllowlisted`、`checkResponseCorrelation`、`dispatchAllowed` 等函数，以及 `NonceStore` / `OutboundSequence` / `InboundSequence` 对象 —— 附带断言 golden-vector 对等的 macOS 本地 smoke。异步节点生命周期（start、listen、`connect`）留在 Rust 侧。Swift 在具备可打包工程后同样发布到 GitHub Packages。

## 目标语言矩阵

目标语言为 C#、Go、Python、Swift、Kotlin（按产品方向优先级）；**Swift 与 C# 已落地** —— C# 经 vendored `uniffi-bindgen-cs` fork（重定向到 uniffi 0.32）实现（[决策记录](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md)）。TypeScript 是并行的**路径 A**（语言直连）轨道。Go / Python / Kotlin 为剩余目标。

## 集成方须知

- **仅核心（core-only）** —— 导出面即会话核心；宿主语言对照线上契约自行实现传输适配器。
- 密钥以原始字节跨 FFI 边界；对等节点标识以字符串；清单 / hello 以 JSON 字符串。
- **C# 集成方**使用 `PackageReference`；维护者在 FFI 面变更时用 vendored bindgen fork 重新生成（见 Smoke README）。

## 规范参考

- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) §嵌入模型 —— 路径 B 定义与纯度规则
- [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md) —— npm/crates.io 与 GitHub Packages 的注册表分工
- [crates/spoke-connect/README.md#binding-facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) —— 同步/异步边界、导出面、目标语言矩阵
- [connect-csharp-binding.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md) —— C# 绑定决策记录
