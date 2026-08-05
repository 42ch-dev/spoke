---
title: 从原生绑定连接
---

# 从原生绑定连接（Connect from native bindings）

**原生绑定（native bindings）**通过 FFI 把共享的 connect **会话核心**嵌入宿主语言：纯会话规则 —— `peer_id` 推导、握手签名/校验、allowlist、nonce 单次使用、sequence 分配、关联校验、dispatch gate —— 集中在一个核心中，传输则留在各宿主语言。绑定由 Rust 参考 crate `spoke-connect` 的核心生成，经**四种渠道类型**覆盖五种语言，全部与 SPOKE git tag `vX.Y.Z` 锁步：

| 语言 | 渠道 | 软件包 |
|------|------|--------|
| C# | GitHub Packages NuGet | `42ch.Spoke.Connect` |
| Kotlin | GitHub Packages Maven | `dev.42ch:spoke-connect` |
| Swift | Swift Package Manager（git + tags） | 产品 `SpokeConnect` |
| Go | Go modules（git + tags） | `github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go` |
| Python | PyPI | `spoke-connect` |

NuGet 与 Maven 共用 GitHub Packages 注册表族。每个绑定暴露相同的同步核心面；golden-parity smoke 从各宿主侧断言字节级一致的行为。

## C# —— GitHub Packages NuGet

```xml
<!-- nuget.config（每个解决方案配置一次） -->
<packageSources>
  <add key="github-42ch" value="https://nuget.pkg.github.com/42ch-dev/index.json" />
</packageSources>

<PackageReference Include="42ch.Spoke.Connect" Version="X.Y.Z" />
```

使用带 `read:packages` 权限的令牌向 GitHub Packages 认证。原生 `libspoke_connect` 随 NuGet `runtimes/<rid>/native/`（`win-x64`、`linux-x64`、`osx-arm64`）发布。

```csharp
using uniffi.spoke_connect;

var peerId = SpokeConnectMethods.DerivePeerIdFromEd25519Pubkey(pubkey);
var version = SpokeConnectMethods.ProtocolVersion(); // 1
```

软件包详情：[`bindings/csharp/PACKAGE.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/csharp/PACKAGE.md)。

## Kotlin —— GitHub Packages Maven

```kotlin
// settings.gradle.kts 或 build.gradle.kts 的 repository 块
maven {
    url = uri("https://maven.pkg.github.com/42ch-dev/spoke")
    credentials {
        username = providers.gradleProperty("gpr.user").get()
        password = providers.gradleProperty("gpr.key").get()
    }
}

dependencies {
    implementation("dev.42ch:spoke-connect:X.Y.Z")
    // JNA 是已发布产物的传递依赖
}
```

在 `gradle.properties` 或 `~/.gradle/gradle.properties` 中设置 `gpr.user` 与 `gpr.key`（GitHub 用户名与带 `read:packages` 的令牌）。JNA 从 jar 加载平台原生库（`darwin-aarch64`、`linux-x86-64`、`win32-x86-64`）。

```kotlin
import uniffi.spoke_connect.derivePeerIdFromEd25519Pubkey
import uniffi.spoke_connect.protocolVersion

val peerId = derivePeerIdFromEd25519Pubkey(pubkey)
val version = protocolVersion() // 1
```

绑定 README：[`bindings/kotlin/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/kotlin/README.md)。

## Swift —— Swift Package Manager

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

在 `vX.Y.Z` tag 上，SPM 解析仓库根 `Package.swift`，得到产品 `SpokeConnect`（生成的 Swift 代码加 `spoke_connectFFI` xcframework）。

```swift
import SpokeConnect

let peerId = try derivePeerIdFromEd25519Pubkey(pubkey: goldenPubkey)
let version = protocolVersion() // 1
```

绑定 README：[`bindings/swift/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/swift/README.md)。

## Go —— Go modules

```bash
go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z
```

在 `vX.Y.Z` tag 上，仓库根 `go.mod`（`module github.com/42ch-dev/spoke`）为模块定版本；导入路径即子目录包。cgo 链接模块树中 `native/<goos>_<goarch>/` 下的共享库；消费者需要 C 工具链与 `CGO_ENABLED=1`（永不需要 Rust 工具链）。

```go
import spokeconnect "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go"

peerID, err := spokeconnect.DerivePeerIdFromEd25519Pubkey(pubkey)
version := spokeconnect.ProtocolVersion() // 1
```

绑定 README：[`bindings/go/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/go/README.md)。

## Python —— PyPI

```bash
pip install spoke-connect==X.Y.Z
```

平台 wheel（`manylinux`、`macosx_11_0_arm64`、`win_amd64`）经发布工作流上的 Trusted Publishing 发布到 PyPI 项目 **`spoke-connect`**。

```python
import spoke_connect

peer_id = spoke_connect.derive_peer_id_from_ed25519_pubkey(pubkey)
version = spoke_connect.protocol_version()  # 1
```

绑定 README：[`bindings/python/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/python/README.md)。

## 共享会话核心

五种绑定暴露同一套同步核心面：`peer_id` 推导、握手签名/校验、allowlist、nonce store、sequence 分配、响应关联、dispatch gate 与协议版本。密钥以原始字节跨 FFI 边界（校验为恰好 32 字节），peer id 以字符串，manifest / 握手信封以 JSON 字符串 —— 传输 adapter 留在宿主语言，按线上契约实现。

TypeScript **语言原生客户端**（[从 TypeScript 客户端连接](/zh/how-to/connect-ts-client)）直接用 TypeScript 实现同一套会话核心规则 —— 它是并行的姊妹路径，不是绑定行。**Rust 参考实现**（crates.io 上的 `spoke-connect`）是会话核心参考与绑定来源；共享契约见[connect 线上参考](/zh/reference/connect)。

## 下一步

- [开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 每个绑定都实现的握手流程。
- [从原生绑定使用 RemoteAdapter](/zh/how-to/remote-adapter-native-binding) —— 拨号 `Transport`、调用 port 方法并在 FFI 上跨对等节点路由。
- [connect 线上参考](/zh/reference/connect) —— 信封字段表与身份绑定。
