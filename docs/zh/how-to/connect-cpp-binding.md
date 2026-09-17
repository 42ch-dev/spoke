---
title: 从 C 与 C++ 连接
---

# 从 C 与 C++ 连接

C 与 C++ 渠道通过手写 C ABI 消费共享的 connect **会话核心**：一个手写头文件 `spoke_connect.h`，加上每个平台提交的动态库。C++17 宿主以 `-fno-exceptions -fno-rtti`（MSVC 为 `/EHs-c-` `/GR-`）编译该头文件并链接载体。该边界通过 `spoke_connect_abi_version` 报告 ABI 修订号 `1`。

载体包装的是生成式**原生绑定**所暴露的同一套 Rust 公共 facade —— 会话核心（`peer_id` 推导、握手签名/校验、allowlist、nonce store、sequence 计数器、响应关联、dispatch gate）、`RemoteAdapter`、`MultiPeerRouter` 与 `ConnectResponder`（含 `PortsHandler` 与 `ToolHandler` 回调），以及内存回环辅助函数。对照表 [`bindings/cpp/parity.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/parity.md) 逐项记录每个生产 facade 成员、回调与错误变体对应的 C 声明。

头文件与各平台原生库都存放在仓库中，并从发布 tag `vX.Y.Z` 一并解析：检出你使用的 tag，并从同一修订获取两者。

## 1. 取得头文件与原生库

| 路径 | 内容 |
|------|------|
| `crates/spoke-connect/bindings/cpp/include/spoke_connect.h` | C 契约：状态值、值类型、回调表与全部导出函数 |
| `crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib` | macOS arm64 载体，安装名 `@rpath/libspoke_connect_capi.dylib` |
| `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll` | Windows x64 载体 |
| `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll.lib` | Windows 链接步骤所用的 Rust 生成导入库 |
| `crates/spoke-connect/bindings/cpp/native/provenance.json` | 每个 RID：源码修订、target、`rustc -Vv`、编译器版本、构建标志、头文件与产物哈希 |
| `crates/spoke-connect/bindings/cpp/README.md` | 绑定 README（完整调用契约） |

头文件以 C99 编译（`<stdint.h>` 的定宽整数、`<stddef.h>` 的长度类型），C++ 通过 `extern "C"` 包含。Windows 上的函数与回调指针使用 `__cdecl`，macOS 使用平台默认 C 调用约定。所有公开符号都以 `spoke_connect_` 为前缀。

## 2. 编译与链接

macOS arm64 链接 dylib，并把加载器指向其目录：

```sh
clang++ -std=c++17 -fno-exceptions -fno-rtti \
  -I crates/spoke-connect/bindings/cpp/include \
  host.cpp crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib \
  -Wl,-rpath,<native/osx-arm64 的绝对路径> \
  -o host
```

Windows x64 链接导入库，以 release C++ CRT（`/MD`）配载体的 Rust 动态 CRT，并从可执行文件所在目录加载 DLL：

```bat
cl.exe /nologo /std:c++17 /EHs-c- /GR- /MD /W4 /WX /I<include 的绝对路径> ^
  host.cpp <native\win-x64\spoke_connect_capi.dll.lib 的绝对路径> /Fe:host.exe
copy /Y <native\win-x64\spoke_connect_capi.dll 的绝对路径> .
host.exe
```

已验证的 Windows 组合是 `/MD`：release C++ CRT 与载体的 Rust 动态 CRT 相匹配。

## 3. 调用 ABI

| 关注点 | 契约 |
|--------|------|
| 状态 | 每次调用返回 `int32_t`：成功为 `SPOKE_CONNECT_OK`（0），否则为某个 `SPOKE_CONNECT_*` 值，细节写在调用方的错误记录中。失败的调用保留调用方对其出参的所有权。 |
| 出参 | 调用前将每个出参与 `SpokeConnectError` 记录清零；库只填充该结果携带的字段。 |
| 所有权结果 | `SpokeConnectBuffer` 用 `spoke_connect_buffer_free` 释放，`SpokeConnectOptionalBuffer` 用 `spoke_connect_optional_buffer_free`，`SpokeConnectError` 用 `spoke_connect_error_free`。 |
| 借用输入 | `SpokeConnectSlice` 在调用期间借用字节；长度是权威，文本按 UTF-8 校验，密钥为 32 原始字节。 |
| 句柄 | 每个句柄拥有一个载体对象，用 `<object>_free` 释放（释放空句柄是幂等的）；`close` 结束会话，释放仍由 `<object>_free` 完成。构造函数与路由器借用传入的句柄。 |
| 回调表 | `spoke_connect_transport_new`、`spoke_connect_ports_handler_new` 与 `spoke_connect_tool_handler_new` 复制表，并在成功时接管 `user_data` 的所有权：其 `destroy` 在最后一个引用与在途回调之后恰好执行一次。 |
| 回传缓冲区 | 回调把已填充的 `SpokeConnectForeignBuffer` 所有权交给载体，载体复制字节并恰好调用一次其 `release`。 |
| 线程 | 调用会阻塞调用方宿主线程；回调在载体的 blocking pool 上运行并可能并发，因此宿主上下文需线程安全。 |
| 生命周期 | 让载体在进程生命周期内保持加载，并在宿主关闭前关闭会话、释放句柄。 |

状态值按失败词汇分组：

| 区间 | 含义 |
|------|------|
| 0 | `SPOKE_CONNECT_OK` |
| 1 / 2 | `SPOKE_CONNECT_INVALID_ARGUMENT`（C 边界校验）/ `SPOKE_CONNECT_PANIC`（被包裹的 panic） |
| 100–107 | 核心错误：握手签名无效、nonce 重放、握手失败、nonce 无效、加密、JCS、令牌无效、协议版本不匹配 |
| 200–202 | 核心 invoke 错误：sequence 耗尽、inbound sequence 不匹配、关联不匹配 |
| 300 / 301 | 拨号错误 / 被拒绝（应用与 dispatch 拒绝；在映射定义处保留 `code`、`kind` 与 `wire_code`） |
| 400 / 401 | 传输关闭 / 传输 I/O |

## 4. 运行 golden-parity smoke

`tooling/connect/cpp-smoke.mjs` 编译仓库中的 `Smoke/main.cpp`，链接所需 RID 的已暂存原生库，并针对共享 golden vector `crates/spoke-connect/tests/fixtures/golden-hello.json` 运行：

```sh
node tooling/connect/cpp-smoke.mjs --rid osx-arm64
node tooling/connect/cpp-smoke.mjs --rid win-x64
```

绿色运行会按顺序为每个断言组打印一行横幅：

```text
golden peer-id: PASS
golden hello signature: PASS
protocol version 1: PASS
loopback ports: PASS
rejection/ownership: PASS
C++ smoke: PASS
```

每行横幅位于其断言组之后；运行在首个失败检查处停止，并以非零状态退出。Windows 车道 [`.github/workflows/cpp-connect.yml`](https://github.com/42ch-dev/spoke/blob/main/.github/workflows/cpp-connect.yml) 会在任何触及载体、其头文件、smoke 或构建/校验/冒烟脚本的 pull request 与 main push 上，用 MSVC 编译、链接并运行同一个 smoke。

## 5. 在 Unreal Engine 中消费该载体

Unreal Engine 模块参考（[`bindings/cpp/ue/SpokeConnect.Build.cs`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/ue/SpokeConnect.Build.cs) 及其 [README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/ue/README.md)）声明一个外部模块：把头文件加入 include 路径，并为目标平台链接与暂存已提交的载体。UnrealBuildTool 在项目或插件的 `Source/` 树中发现模块，因此请把 `cpp/` 树整体放入，例如 `MyProject/Source/SpokeConnect/`，其中 `ue/SpokeConnect.Build.cs` 与 `include/`、`native/` 并列；再把该模块加入消费模块的依赖：

```csharp
PublicDependencyModuleNames.AddRange(new string[] { "SpokeConnect" });
```

| 目标 | 链接 | 运行时暂存 |
|------|------|------------|
| `Win64`（x86_64） | 通过 `PublicAdditionalLibraries` 链接 `native/win-x64/spoke_connect_capi.dll.lib` | `RuntimeDependencies.Add("$(TargetOutputDir)/spoke_connect_capi.dll", …)` 把 DLL 复制到可执行文件旁 |
| `Mac`（arm64） | 通过 `PublicAdditionalLibraries` 链接 `native/osx-arm64/libspoke_connect_capi.dylib` | `RuntimeDependencies.Add(…, StagedFileType.NonUFS)` 以松散文件把 dylib 放在可执行文件旁 |

该模块接受 `Win64` + `x86_64` 与 `Mac` + `arm64`，经 `UnrealArch` API 读取目标架构（UE 5.2 起），并贡献 include 路径与链接/暂存接线 —— 消费模块自行调用所需的 C ABI 函数、拥有传输实现，并决定由哪个引擎线程消费回调结果。载体调用会阻塞调用方 OS 线程，因此请从工作线程调用 ABI；回调在载体的 blocking pool 上到达并可能并发。让载体在进程生命周期内保持加载，先关闭会话，再在宿主关闭前释放句柄。

验证状态：已执行的证据是 `osx-arm64` 与 `win-x64` 上的独立 C++17 smoke（第 4 步）；模块 README 列出的引擎侧检查 —— 编辑器构建与加载、打包后松散库加载、一次真实连接会话、有序关闭、目标设置 —— 是持有引擎环境的维护者所运行的清单。

## 6. 让头文件与导出保持同步

`tooling/connect/cpp-symbol-check.mjs` 是可执行的漂移门：解析头文件的声明块，与载体导出的 `spoke_connect_*` 符号双向比对，把头文件的 `typedef struct` 块与载体报告的记录布局对照，编译一个对每条声明都持有类型化函数指针的 C99 探针，并编译一个使用 `-fno-exceptions -fno-rtti` 的 C++17 包含检查。

```sh
node tooling/connect/cpp-symbol-check.mjs \
  --header crates/spoke-connect/bindings/cpp/include/spoke_connect.h \
  --library crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib
```

```text
C ABI symbols: PASS (83 declarations, 83 exports, 0 missing, 0 extra)
Record layout: 11 records, 66 assertions (sizeof/_Alignof/offsetof) match the carrier mirrors
C probe (clang -std=c99 -Wall -Wextra -Werror): PASS
C++17 inclusion (-fno-exceptions -fno-rtti): PASS
```

`C++17 inclusion` 行用精确的编译旗标等价重述了该漂移门的检查标签。

## 下一步

- [把原生绑定接到 RemoteAdapter](/zh/how-to/remote-adapter-native-binding) —— C ABI 所携带的 adapter、路由器、响应方、ports 与工具面。
- [connect 线上参考](/zh/reference/connect) —— 信封字段表与身份绑定。
- [开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 每个绑定都实现的握手流程。
