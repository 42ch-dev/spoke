---
title: 原生绑定
---

# 原生绑定（Native bindings，Path B）

路径 B 将**共享会话核心**经 FFI 嵌入宿主语言：纯同步的会话规则（`peer_id`（对等节点标识）派生、hello 签名/验证、allowlist、nonce 一次性使用、序列分配、关联、dispatch gate）全部位于一个核心中，传输留在各宿主语言。参考实现是 `spoke-connect` crate 的 **Binding facade**。

本页为[英文原页](/connect/bindings)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## Binding facade

crate 的 `src/core/` 是纯同步、语言可移植层 —— libp2p、tokio 与 I/O 位于传输层，传输层在边界处转换 `libp2p::PeerId` ↔ `String`。facade 决策记录在 crate README 的 [Binding facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) 一节，含同步/异步边界与导出面。

## 发布渠道

路径 B 绑定通过**四种渠道类型**（五种语言）发布，均与 spoke 标签 `vX.Y.Z` 锁步：

| 渠道 | 语言 | 集成方入口 |
|------|------|------------|
| **GitHub Packages NuGet** | C# | `42ch.Spoke.Connect` |
| **GitHub Packages Maven** | Kotlin | `io.github.42ch-dev:spoke-connect` |
| **Swift Package Manager**（git + 标签） | Swift | 根目录 `Package.swift` — `.package(url:from:)` |
| **Go modules**（git + 标签） | Go | `go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z` |
| **PyPI**（Trusted Publishing） | Python | `pip install spoke-connect` |

NuGet 与 Maven 共用 **GitHub Packages** 渠道类型（同一注册表家族，两种包生态）。

打包坐标与原生库布局：[connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md)。发布阶段与注册表鉴权：[connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md)。

## C# NuGet（`42ch.Spoke.Connect`）

集成方通过 **GitHub Packages NuGet** 消费会话核心：

```xml
<!-- nuget.config（每个解决方案配置一次） -->
<packageSources>
  <add key="github-42ch" value="https://nuget.pkg.github.com/42ch-dev/index.json" />
</packageSources>
```

```xml
<PackageReference Include="42ch.Spoke.Connect" Version="0.7.1" />
```

使用具备 `read:packages` 权限的 PAT（或 Actions 中的 `GITHUB_TOKEN`）向 GitHub Packages 鉴权。原生 `spoke_connect` / `libspoke_connect` 位于 NuGet `runtimes/<rid>/native/`（`win-x64`、`linux-x64`、`osx-arm64`）。包版本与 spoke 锁步 SemVer / 标签 `vX.Y.Z` 对齐。

## Kotlin Maven（`io.github.42ch-dev:spoke-connect`）

集成方添加 GitHub Packages Maven 仓库，并在 spoke 锁步 SemVer `X.Y.Z` 处依赖绑定构件：

```kotlin
// settings.gradle.kts 或 build.gradle.kts 仓库块
maven {
    url = uri("https://maven.pkg.github.com/42ch-dev/spoke")
    credentials {
        username = providers.gradleProperty("gpr.user").get()
        password = providers.gradleProperty("gpr.key").get()
    }
}
```

```kotlin
dependencies {
    implementation("io.github.42ch-dev:spoke-connect:X.Y.Z")
    // JNA 为发布构件的传递依赖
}
```

```kotlin
import uniffi.spoke_connect.derivePeerIdFromEd25519Pubkey
import uniffi.spoke_connect.protocolVersion

val peerId = derivePeerIdFromEd25519Pubkey(pubkey)
val version = protocolVersion() // 1
```

在 `gradle.properties` 或 `~/.gradle/gradle.properties` 中设置 `gpr.user` 与 `gpr.key`（GitHub 用户名及具备 `read:packages` 的 PAT）。稳定标签经 [`release.yml`](https://github.com/42ch-dev/spoke/blob/main/.github/workflows/release.yml) 的 `publish-maven` 发布。

JNA 按 Maven 布局契约从 jar 加载平台原生库（`darwin-aarch64`、`linux-x86-64`、`win32-x86-64`）。版本与 spoke 标签 `vX.Y.Z` 锁步。绑定 README：[`bindings/kotlin/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/kotlin/README.md)。

## Swift（经 SPM 的 `SpokeConnect`）

集成方在锁步 SemVer `X.Y.Z` 处将 spoke 仓库添加为 SPM 依赖：

```swift
// Package.swift
dependencies: [
    .package(url: "https://github.com/42ch-dev/spoke.git", from: "X.Y.Z"),
],
targets: [
    .target(
        name: "MyApp",
        dependencies: [
            .product(name: "SpokeConnect", package: "spoke"),
        ]
    ),
]
```

在标签 `vX.Y.Z` 处，SPM 解析仓库根目录 `Package.swift`，取得库产品 `SpokeConnect`、生成 Swift 与 `spoke_connectFFI` xcframework（按打包契约）。

```swift
import SpokeConnect

let peerId = try derivePeerIdFromEd25519Pubkey(pubkey: goldenPubkey)
let version = protocolVersion() // 1
```

绑定 README：[`bindings/swift/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/swift/README.md)。

## Go（`github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go`）

集成方在 spoke 锁步标签 `vX.Y.Z` 处固定绑定模块：

```bash
go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z
```

```go
import spokeconnect "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go"
```

在标签 `vX.Y.Z` 处，仓库根目录 `go.mod`（`module github.com/42ch-dev/spoke`）为模块定版；cgo 按打包契约从 `native/<goos>_<goarch>/` 加载原生库。当前已提交的原生库：`darwin_arm64`、`darwin_amd64`；`linux_amd64` 与 `windows_amd64` 待维护者从 `build-connect-ffi` 产物落盘（见 [`crates/spoke-connect/bindings/go/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/go/README.md)）。集成方需 C 工具链且 `CGO_ENABLED=1`。

## Python（PyPI）

集成方在 spoke 锁步 SemVer `X.Y.Z` 处从 PyPI 安装：

```bash
pip install spoke-connect==X.Y.Z
```

```python
import spoke_connect

peer_id = spoke_connect.derive_peer_id_from_ed25519_pubkey(pubkey)
```

平台 wheel（`manylinux_2_17_x86_64`、`macosx_11_0_arm64`、`win_amd64`）经顶层 `release.yml` 工作流上的 Trusted Publishing OIDC（`publish-pypi` 任务，仓库 `42ch-dev/spoke`）发布至 PyPI 项目 **`spoke-connect`**。版本与 spoke 标签 `vX.Y.Z` 锁步。见 [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.3。

## 目标语言矩阵

目标语言为 C#、Go、Python、Swift、Kotlin（按产品方向优先级）。**C#**（NuGet）、**Swift**（SPM `SpokeConnect`）、**Kotlin**（GitHub Packages Maven）、**Go**（Go modules + 黄金向量 smoke）与 **Python**（PyPI 平台 wheel + 黄金向量 smoke）**已落地** —— C# 与 Go 经 vendored uniffi bindgen fork（重定向到 uniffi 0.32）实现（[C# 决策记录](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md)；Go 见 [`bindings/go/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/go/README.md)；Swift 见 [`bindings/swift/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/swift/README.md)；Kotlin 见 [`bindings/kotlin/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/kotlin/README.md)；Python 见 [`bindings/python/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/python/README.md)）。TypeScript 是并行的**路径 A**（语言直连）轨道。

## 集成方须知

- **仅核心（core-only）** —— 导出面即会话核心；宿主语言对照线上契约自行实现传输适配器。
- 密钥以原始字节跨 FFI 边界；对等节点标识以字符串；清单 / hello 以 JSON 字符串。
- **C# 集成方**使用 `PackageReference`；维护者在 FFI 面变更时用 vendored bindgen fork 重新生成（见 Smoke README）。

## 规范参考

- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) §嵌入模型 —— 路径 B 定义与纯度规则
- [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md) —— 发布阶段与四渠道注册表分工
- [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) —— 各语言打包契约（坐标、原生库、CI 任务）
- [crates/spoke-connect/README.md#binding-facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) —— 同步/异步边界、导出面、目标语言矩阵
- [connect-csharp-binding.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md) —— C# 绑定决策记录
