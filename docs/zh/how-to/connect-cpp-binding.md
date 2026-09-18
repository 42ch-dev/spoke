---
title: 从 C 与 C++ 连接
---

# 从 C 与 C++ 连接

C/C++ 渠道提供两个手写头文件与每个平台一个已提交载体。`spoke_connect.h` 是 C 契约 —— C99，包含状态值、值类型、回调表与全部导出函数，并通过 `spoke_connect_abi_version` 报告 ABI 修订号 `1`。`spoke_connect.hpp` 是覆盖该契约的 header-only C++17 便利层：命名空间 `spoke::connect`，为每个受拥有的缓冲区与句柄提供 move-only 所有权、借用文本视图与显式拷贝、单一结构化 `Result` 错误通道，以及宿主回调桥。C++17 宿主包含 `spoke_connect.hpp`（它包含 C 头文件）并链接原生载体。

已提交的 C/C++ 载体面向 macOS arm64（`osx-arm64`）与 Windows x64（`win-x64`）。请从同一仓库 tag `vX.Y.Z` 取得 `spoke_connect.h`、`spoke_connect.hpp` 与你的目标平台的原生文件。

消费方默认以禁用异常与 RTTI 编译：clang `-std=c++17 -fno-exceptions -fno-rtti`，MSVC `/std:c++17 /EHs-c- /GR- /MD /D_HAS_EXCEPTIONS=0`。启用异常的构建使用同样的错误 API —— clang `-std=c++17 -fexceptions -fno-rtti`，MSVC `/std:c++17 /EHsc /GR- /MD` —— 并且在同一个链接映像中包含便利头文件的每个翻译单元必须使用一致的异常与标准库配置。

载体包装的是生成式**原生绑定**所暴露的同一套 Rust 公共 facade —— 会话核心（`peer_id` 推导、握手签名/校验、allowlist、nonce store、sequence 计数器、响应关联、dispatch gate）、`RemoteAdapter`、`MultiPeerRouter` 与 `ConnectResponder`（含 `PortsHandler` 与 `ToolHandler` 回调），以及内存回环辅助函数。对照表 [`bindings/cpp/parity.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/parity.md) 逐项记录每个生产 facade 成员、回调与错误变体对应的 C 声明，并记录消费该声明的 C++ 对应面。`PortsHandler` 回调桥接 port 目录与可选的 `extract` 服务面；其方法列表见 connect 线上参考中的[服务（响应方）](/zh/reference/connect#服务-响应方)。

## 1. 取得头文件与原生库

| 路径 | 内容 |
|------|------|
| `crates/spoke-connect/bindings/cpp/include/spoke_connect.h` | C 契约：状态值、值类型、回调表与全部导出函数 |
| `crates/spoke-connect/bindings/cpp/include/spoke_connect.hpp` | C++17 便利层：`Buffer`、`Error`、`Result<T>`、各句柄类、三种回调记录及其工厂 |
| `crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib` | macOS arm64 载体，安装名 `@rpath/libspoke_connect_capi.dylib` |
| `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll` | Windows x64 载体 |
| `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll.lib` | Windows 链接步骤所用的 Rust 生成导入库 |
| `crates/spoke-connect/bindings/cpp/native/provenance.json` | 每个 RID：源码修订、target、`rustc -Vv`、编译器版本、构建标志、头文件与产物哈希 |
| `crates/spoke-connect/bindings/cpp/README.md` | 绑定 README（完整调用契约） |

C 头文件以 C99 编译（`<stdint.h>` 的定宽整数、`<stddef.h>` 的长度类型），C++ 通过 `extern "C"` 包含。Windows 上的函数与回调指针使用 `__cdecl`，macOS 使用平台默认 C 调用约定。所有公开符号都以 `spoke_connect_` 为前缀。便利头文件只依赖 C++17 标准库，且不定义自己的任何配置宏。

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
cl.exe /nologo /std:c++17 /EHs-c- /GR- /MD /D_HAS_EXCEPTIONS=0 /W4 /WX ^
  /I<include 的绝对路径> ^
  host.cpp <native\win-x64\spoke_connect_capi.dll.lib 的绝对路径> /Fe:host.exe
copy /Y <native\win-x64\spoke_connect_capi.dll 的绝对路径> .
host.exe
```

已验证的 Windows 组合是 `/MD`：release C++ CRT 与载体的 Rust 动态 CRT 相匹配。`/D_HAS_EXCEPTIONS=0` 是免异常配置中标准库的那一半；启用异常的构建把 `/EHs-c- /D_HAS_EXCEPTIONS=0` 换成 `/EHsc`，并把 macOS 旗标换成 `-fexceptions -fno-rtti`。

## 3. 建立一个会话

下面三个代码块组成一个完整的 `host.cpp`：宿主自有的队列传输、带所服务 echo 工具的演示身份、以及拨号与一次真实调用。按顺序拼接即可 —— 用到的每个 include 与辅助函数都已列出。服务句柄出现之前的失败会带着底层 status 结束进程；`ConnectResponder::serve` 返回之后的每一次失败都会先打印错误、走有序关闭，并作为进程的退出状态返回。

### 第 1 段 —— 宿主传输

```cpp
// ── host.cpp — append part 1 of 3 ───────────────────────────────────────────
//
// The host transport: the includes, the error-printing and construction
// helpers, and the queue pair whose callbacks carry every envelope this session
// sends and receives.

#include "spoke_connect.hpp"

#include <array>
#include <condition_variable>
#include <cstdint>
#include <cstdlib>
#include <deque>
#include <iostream>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>
#include <utility>

namespace connect = spoke::connect;
using connect::Buffer;
using connect::Error;
using connect::Result;

/**
 * Prints one failure and returns the status the example exits with — the first
 * status reported, so a later failure never masks an earlier one.
 */
int report(int previous, const char* step, const Error& error) {
    std::cerr << step << ": status " << error.status << " — " << error.message;
    if (error.code.has_value()) std::cerr << " (code " << *error.code << ")";
    std::cerr << "\n";
    return previous == SPOKE_CONNECT_OK ? error.status : previous;
}

/**
 * Returns the value of a construction step that runs before a serving handle
 * exists; a failure prints the error and ends the process with its status.
 * Every step that runs after `ConnectResponder::serve` has returned propagates
 * its failure instead, so the session this example acquires always leaves
 * through the ordered close path.
 */
template <typename T>
T unwrap(Result<T>&& result, const char* step) {
    if (result.has_value()) return std::move(result).value();
    std::cerr << step << ": status " << result.error().status << " — " << result.error().message
              << "\n";
    std::exit(result.error().status);
}

/** One direction of the host queue pair: an unbounded FIFO with a close flag. */
class EnvelopeQueue {
  public:
    /**
     * Enqueues one envelope. A push into a closed queue reports failure, so a
     * send that can no longer be delivered is never dropped silently.
     */
    bool push(std::string envelope) {
        std::lock_guard<std::mutex> guard(mutex_);
        if (closed_) return false;
        envelopes_.push_back(std::move(envelope));
        signal_.notify_all();
        return true;
    }

    /** Blocks the calling thread until an envelope arrives or the queue closes. */
    bool pop(std::string* out) {
        std::unique_lock<std::mutex> lock(mutex_);
        for (;;) {
            if (!envelopes_.empty()) {
                *out = std::move(envelopes_.front());
                envelopes_.pop_front();
                return true;
            }
            if (closed_) return false;
            signal_.wait(lock);
        }
    }

    /** Closes the queue and wakes every waiter, so a blocked `recv` returns. */
    void close() {
        std::lock_guard<std::mutex> guard(mutex_);
        closed_ = true;
        signal_.notify_all();
    }

  private:
    std::mutex mutex_;
    std::condition_variable signal_;
    std::deque<std::string> envelopes_;
    bool closed_ = false;
};

/** One end's view of the pair: what it sends on and what it receives from. */
struct HostQueues {
    std::shared_ptr<EnvelopeQueue> outbound;
    std::shared_ptr<EnvelopeQueue> inbound;

    /** Closes both directions; `close` on either direction is idempotent. */
    void close() const {
        outbound->close();
        inbound->close();
    }
};

/** The transport-closed failure a closed queue reports. */
Error transport_closed(std::string_view message) {
    Error error;
    error.status = SPOKE_CONNECT_TRANSPORT_CLOSED;
    error.message = std::string(message);
    return error;
}

/**
 * Builds one end's transport record over its queue pair. Every callback
 * captures the shared state by value, so the queues outlive the record, and no
 * callback calls back into an operational C or C++ API — the exported loopback
 * helpers included. Callbacks run on the carrier's blocking pool and may run
 * concurrently, which the queue's own lock covers.
 */
std::unique_ptr<connect::TransportCallbacks> make_transport(HostQueues host) {
    auto callbacks = std::make_unique<connect::TransportCallbacks>();
    callbacks->send = [host](std::string_view envelope) -> Result<void> {
        if (!host.outbound->push(std::string(envelope))) {
            return Result<void>::failure(transport_closed("the host queue is closed"));
        }
        return Result<void>::success();
    };
    callbacks->recv = [host]() -> Result<std::string> {
        std::string envelope;
        if (!host.inbound->pop(&envelope)) {
            return Result<std::string>::failure(transport_closed("the host queue is closed"));
        }
        return Result<std::string>::success(std::move(envelope));
    };
    callbacks->close = [host]() -> Result<void> {
        host.close();
        return Result<void>::success();
    };
    return callbacks;
}
```

### 第 2 段 —— 身份与服务端

```cpp
// ── host.cpp — append part 2 of 3 ───────────────────────────────────────────
//
// The demo identity, the capability manifest both ends advertise, and the
// serving end: both transports, `ConnectResponder::serve`, the echo tool, and
// the ordered cleanup every exit path shares.

/** The tool this example serves, and the arguments the dialer invokes it with. */
constexpr std::string_view kEchoToolId = "tools.example.echo";
constexpr std::string_view kEchoArguments = R"({"message":"hello"})";

/**
 * Local demo data: the 32-byte Ed25519 seed and its public key from the
 * repository's shared golden hello vector
 * (`crates/spoke-connect/tests/fixtures/golden-hello.json`). This example runs
 * both ends in one process and gives them the same identity, so the signed
 * hello is self-contained; a production host generates and stores its own
 * identity and takes the remote public key from the peer it dials.
 */
const std::array<uint8_t, 32> kDemoSeed = {
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
    0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20};
const std::array<uint8_t, 32> kDemoPubkey = {
    0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9, 0x40, 0x78, 0xb1, 0x12, 0xe8, 0xa9, 0x8b, 0xa7,
    0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7, 0xe0, 0xe3, 0x91, 0x0b, 0xad, 0x04, 0x96, 0x64};

/**
 * The capability manifest both ends advertise. `capabilities` carries the
 * connect capability, the baseline capability and the echo tool's own
 * capability string — a `tools.<ns>.<tool_id>` op requires exactly that string
 * to be negotiated, and a capability enters the negotiated set only when both
 * hellos list it — while `namespaces` owns the `example` namespace the tool
 * descriptor declares.
 */
const char* const kManifest = R"json({
  "capabilities": ["spoke-connect", "spoke-baseline", "tools.example.echo"],
  "extensions": {},
  "host_id": "cpp-example-host",
  "namespaces": ["example"],
  "roles": ["data-store"],
  "schema_version": 1,
  "tools": [
    {
      "capability_id": "tools.example.echo",
      "description": "Echo the arguments",
      "input": { "type": "object" },
      "op": "tools.example.echo",
      "output": { "type": "object" },
      "schema_version": 1
    }
  ]
})json";

/** The echo tool record: the handler answers with an owned copy of the arguments. */
std::unique_ptr<connect::ToolCallbacks> make_echo_tool() {
    auto callbacks = std::make_unique<connect::ToolCallbacks>();
    callbacks->handle = [](std::string_view arguments) -> Result<std::string> {
        return Result<std::string>::success(std::string(arguments));
    };
    return callbacks;
}

/** Everything the process keeps alive across the session. */
struct Endpoints {
    connect::Transport server_transport;
    connect::Transport dialer_transport;
    connect::ConnectResponder responder;
    connect::ToolHandler echo;
    std::string peer_id;
};

/**
 * Ends the session in the documented order — the dialing session when one
 * exists, then the serving session, then both host queues so a blocked `recv`
 * returns — and returns the status the example exits with. Every path that has
 * acquired a session leaves through this one cleanup, so no exit path ends the
 * process while a session is live. A null session is one that was never
 * established or is already closed; the handles themselves release through RAII
 * when their scope ends.
 */
int close_ordered(int previous, const connect::RemoteAdapter* dialer,
                  const connect::ConnectResponder* responder, const HostQueues& server_queues,
                  const HostQueues& dialer_queues) {
    int status = previous;
    if (dialer != nullptr) {
        Result<void> closed = dialer->close();
        if (!closed.has_value()) status = report(status, "RemoteAdapter::close", closed.error());
    }
    if (responder != nullptr) {
        Result<void> closed = responder->close();
        if (!closed.has_value()) status = report(status, "ConnectResponder::close", closed.error());
    }
    server_queues.close();
    dialer_queues.close();
    return status;
}

/**
 * Creates both ends over the cross-wired queue pair, starts serving and
 * registers the echo tool. `ports` is absent — the service is offered no ports
 * callbacks at all, which is different from a provider that declines a method.
 *
 * A failure that follows `ConnectResponder::serve` is reported, closed in order
 * and returned to `main`: the serving handle this function acquired is never
 * left to the process to reclaim.
 */
Result<Endpoints> start_endpoints(const HostQueues& server_queues,
                                  const HostQueues& dialer_queues) {
    const SpokeConnectSlice seed = connect::bytes(kDemoSeed.data(), kDemoSeed.size());
    const SpokeConnectSlice pubkey = connect::bytes(kDemoPubkey.data(), kDemoPubkey.size());
    const std::string manifest = kManifest;

    // The peer id is derived from the public key; the allowlist and the
    // peer-key table below are keyed by it.
    Buffer derived = unwrap(connect::derive_peer_id_from_ed25519_pubkey(pubkey),
                            "derive_peer_id_from_ed25519_pubkey");
    std::string peer_id_text = derived.str();  // an owned copy: view() is borrowed
    const SpokeConnectSlice peer_id = connect::slice(peer_id_text);
    const SpokeConnectSlice allowlist[] = {peer_id};
    const SpokeConnectPeerKey peer_keys[] = {SpokeConnectPeerKey{peer_id, pubkey}};
    const std::optional<uint64_t> invoke_timeout = uint64_t{5000};

    std::unique_ptr<connect::TransportCallbacks> server_callbacks =
        make_transport(server_queues);
    connect::Transport server_transport =
        unwrap(connect::Transport::create(server_callbacks), "Transport::create (server)");

    connect::ConnectResponder responder = unwrap(
        connect::ConnectResponder::serve(server_transport, seed, manifest, allowlist, 1,
                                         peer_keys, 1, nullptr, invoke_timeout),
        "ConnectResponder::serve");

    // A serving handle exists from here on: a later failure reports its step,
    // reaches the ordered close, and returns the failure to `main`.
    auto failed = [&](const char* step, Error error) -> Result<Endpoints> {
        error.status = close_ordered(report(SPOKE_CONNECT_OK, step, error), nullptr, &responder,
                                     server_queues, dialer_queues);
        return Result<Endpoints>::failure(std::move(error));
    };

    std::unique_ptr<connect::ToolCallbacks> echo_callbacks = make_echo_tool();
    Result<connect::ToolHandler> echo = connect::ToolHandler::create(echo_callbacks);
    if (!echo.has_value()) return failed("ToolHandler::create", std::move(echo).error());

    Result<void> registered = responder.register_tool_handler(kEchoToolId, echo.value());
    if (!registered.has_value()) {
        return failed("ConnectResponder::register_tool_handler", std::move(registered).error());
    }

    std::unique_ptr<connect::TransportCallbacks> dialer_callbacks = make_transport(dialer_queues);
    Result<connect::Transport> dialer = connect::Transport::create(dialer_callbacks);
    if (!dialer.has_value()) return failed("Transport::create (dialer)", std::move(dialer).error());

    return Result<Endpoints>::success(
        Endpoints{std::move(server_transport), std::move(dialer).value(), std::move(responder),
                  std::move(echo).value(), std::move(peer_id_text)});
}
```

### 第 3 段 —— 拨号、一次调用、关闭

```cpp
// ── host.cpp — append part 3 of 3 ───────────────────────────────────────────
//
// The dialing end: connect (the carrier signs and verifies the hello), one real
// tool invoke, and the ordered close with RAII release.

int main() {
    // The host side: one queue per direction, cross-wired so each end's
    // outbound queue is the other end's inbound queue.
    const auto to_server = std::make_shared<EnvelopeQueue>();
    const auto to_dialer = std::make_shared<EnvelopeQueue>();
    const HostQueues server_queues{to_server, to_dialer};
    const HostQueues dialer_queues{to_dialer, to_server};

    // A failed startup step has already run the ordered close for everything it
    // acquired, so only its status is left to return.
    Result<Endpoints> started = start_endpoints(server_queues, dialer_queues);
    if (!started.has_value()) return started.error().status;
    Endpoints endpoints = std::move(started).value();

    const SpokeConnectSlice seed = connect::bytes(kDemoSeed.data(), kDemoSeed.size());
    const SpokeConnectSlice pubkey = connect::bytes(kDemoPubkey.data(), kDemoPubkey.size());
    const SpokeConnectSlice allowlist[] = {connect::slice(endpoints.peer_id)};
    const std::string manifest = kManifest;

    // `connect` performs the signed hello and the handshake itself — no second
    // hello is sent by hand. Both ends share this example's identity, so the
    // dialed peer is this host. A failed dial is reported and leaves through the
    // same ordered close as every other path; no dialing session exists yet, so
    // the serving session and the host queues are what closes.
    Result<connect::RemoteAdapter> dialed =
        connect::RemoteAdapter::connect(endpoints.dialer_transport, seed, manifest, pubkey,
                                        allowlist, 1, uint64_t{5000});
    if (!dialed.has_value()) {
        return close_ordered(report(SPOKE_CONNECT_OK, "RemoteAdapter::connect", dialed.error()),
                             nullptr, &endpoints.responder, server_queues, dialer_queues);
    }
    connect::RemoteAdapter adapter = std::move(dialed).value();

    int status = SPOKE_CONNECT_OK;

    // A present session id is what an established session carries; the state
    // names the transition the dialer reached.
    Result<Buffer> state = adapter.state();
    if (!state.has_value()) {
        return close_ordered(report(status, "RemoteAdapter::state", state.error()), &adapter,
                             &endpoints.responder, server_queues, dialer_queues);
    }
    Result<std::optional<Buffer>> session = adapter.session_id();
    const bool established = state.value().view() == "Established" && session.has_value() &&
                             session.value().has_value() && !session.value()->empty();
    if (established) {
        std::cout << "state: " << state.value().view()
                  << ", session id: " << session.value()->view() << "\n";
    } else {
        std::cerr << "the session did not establish (state \"" << state.value().view() << "\")\n";
        status = SPOKE_CONNECT_HANDSHAKE_FAILED;
    }

    // One real call: the serving end's registered echo tool answers with the
    // arguments JSON, and the returned `Buffer` owns the result.
    if (status == SPOKE_CONNECT_OK) {
        Result<Buffer> echo = adapter.invoke_tool(kEchoToolId, kEchoArguments);
        if (echo.has_value()) {
            std::cout << kEchoToolId << " -> " << echo.value().view() << "\n";
        } else {
            status = report(status, "RemoteAdapter::invoke_tool", echo.error());
        }
    }

    // The close every path above shares — the success run and the failures
    // alike: end both sessions explicitly, then close the host queues so a
    // blocked `recv` returns, then let this scope release the RAII handles and
    // every buffer it still holds.
    return close_ordered(status, &adapter, &endpoints.responder, server_queues, dialer_queues);
}
```

该程序成功运行时打印：

```text
state: Established, session id: connect-responder-session-12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf
tools.example.echo -> {"message":"hello"}
```

### 这些值保证什么

| 关注点 | 契约 |
|--------|------|
| 借用文本 | `Buffer::view()` 借用所拥有的记录 —— 在该 `Buffer` 存活期间有效，保留二进制长度（内嵌 NUL 字节也是内容），且对临时缓冲区被禁用（其字节随它一起消亡）。空缓冲区读作空视图。 |
| 拥有文本 | `Buffer::str()` 生成一个受拥有的 `std::string`；当值必须比缓冲区活得更久时使用它，上面推导 peer id 即是如此。 |
| 结果 | 每个可失败调用返回 `[[nodiscard]] Result<T>` 或 `Result<void>`，承载原始 `SPOKE_CONNECT_*` 状态与完整结构化 `Error`（`message`、可选的 `code` / `kind` / `wire_code`、`expected`、`actual`）。非零 C 状态一律按值返回。 |
| 裸值 | 句柄与缓冲区是 move-only：析构函数只释放，被移动后的对象为空，`get()` / `release()` / `adopt()` 供与裸 C 互操作。 |
| 会话结束 | 析构函数只释放 —— 从不关闭。`RemoteAdapter::close()` 与 `ConnectResponder::close()` 显式结束会话，宿主自己的传输关闭则释放队列；先关闭，再让作用域释放句柄。 |
| 异常 | 在启用异常的构建中，该层会收容逸出的宿主回调异常：传输回调变为 `SPOKE_CONNECT_TRANSPORT_IO`，ports 或工具回调变为 `SPOKE_CONNECT_FFI_REJECTED` 且 `code="INTERNAL_ERROR"`。禁用异常时头文件中完全没有 `try`、`catch` 或 `throw`，回调通过其 `Result` 报告失败。 |
| 真实网络 | 三个传输回调是唯一的传输接缝：把队列对替换为宿主的网络传输 —— 每次发送一个信封、`recv` 阻塞接收、`close` 唤醒挂起的接收 —— 其上的一切保持不变。导出的回环辅助函数是独立的进程内演示，绝不会被包装成回调传输。 |

## 4. 直接调用 C ABI

便利层是可选的：`spoke_connect.h` 才是契约，C 宿主直接使用它。

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

## 5. 运行 golden-parity smoke

`tooling/connect/cpp-smoke.mjs` 编译仓库中的 `Smoke/main.cpp`（裸 C ABI）与 `Smoke/convenience.cpp`（便利层）两个翻译单元，链接所需 RID 的已暂存原生库，并针对共享 golden vector `crates/spoke-connect/tests/fixtures/golden-hello.json` 运行：

```sh
node tooling/connect/cpp-smoke.mjs --rid osx-arm64
node tooling/connect/cpp-smoke.mjs --rid win-x64
```

在同一 RID 内，运行器构建两种配置 —— 禁用异常（消费方默认）与启用异常 —— 并要求每种配置的横幅行与其期望列表完全相等：数量、顺序与文本都一致。绿色运行会按顺序打印：

```text
golden peer-id: PASS
golden hello signature: PASS
protocol version 1: PASS
loopback ports: PASS
rejection/ownership: PASS
C++ convenience values/core: PASS
C++ convenience callbacks/session: PASS
C++ convenience router: PASS
C++ smoke: PASS
```

启用异常的配置会在最后一行之前额外打印 `C++ callback exception containment: PASS`，因为只有该构建注入抛异常的宿主回调并观察到收容行。运行器以 `C++ smoke runner: PASS (rid <rid>)` 结束；缺失、多余、重复或乱序的横幅都会使运行失败。

Windows 车道 [`.github/workflows/cpp-connect.yml`](https://github.com/42ch-dev/spoke/blob/main/.github/workflows/cpp-connect.yml) 会在任何触及载体、其头文件、smoke 或构建/校验/冒烟脚本的 pull request 与 main push 上，用 MSVC 编译、链接并运行同一个 smoke。

## 6. 在 Unreal Engine 中消费该载体

Unreal Engine 模块参考（[`bindings/cpp/ue/SpokeConnect.Build.cs`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/ue/SpokeConnect.Build.cs) 及其 [README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/ue/README.md)）声明一个外部模块：把头文件目录加入 include 路径，并为目标平台链接与暂存已提交的载体。UnrealBuildTool 在项目或插件的 `Source/` 树中发现模块，因此请把 `cpp/` 树整体放入，例如 `MyProject/Source/SpokeConnect/`，其中 `ue/SpokeConnect.Build.cs` 与 `include/`、`native/` 并列；再把该模块加入消费模块的依赖：

```csharp
PublicDependencyModuleNames.AddRange(new string[] { "SpokeConnect" });
```

| 目标 | 链接 | 运行时暂存 |
|------|------|------------|
| `Win64`（x86_64） | 通过 `PublicAdditionalLibraries` 链接 `native/win-x64/spoke_connect_capi.dll.lib` | `RuntimeDependencies.Add("$(TargetOutputDir)/spoke_connect_capi.dll", …)` 把 DLL 复制到可执行文件旁 |
| `Mac`（arm64） | 通过 `PublicAdditionalLibraries` 链接 `native/osx-arm64/libspoke_connect_capi.dylib` | `RuntimeDependencies.Add(…, StagedFileType.NonUFS)` 以松散文件把 dylib 放在可执行文件旁 |

该模块接受 `Win64` + `x86_64` 与 `Mac` + `arm64`，经 `UnrealArch` API 读取目标架构（UE 5.2 起），并贡献 include 路径与链接/暂存接线 —— 消费模块自行调用所需的 C ABI 函数、拥有传输实现，并决定由哪个引擎线程消费回调结果。载体调用会阻塞调用方 OS 线程，因此请从工作线程调用 ABI；回调在载体的 blocking pool 上到达并可能并发。让载体在进程生命周期内保持加载，先关闭会话，再在宿主关闭前释放句柄。

验证状态：已执行的证据是 `osx-arm64` 与 `win-x64` 上的独立 C++17 smoke（第 5 步）；模块 README 列出的引擎侧检查 —— 编辑器构建与加载、打包后松散库加载、一次真实连接会话、有序关闭、目标设置 —— 是持有引擎环境的维护者所运行的清单。

## 7. 让头文件与导出保持同步

`tooling/connect/cpp-symbol-check.mjs` 是可执行的漂移门：解析头文件的声明块，与载体导出的 `spoke_connect_*` 符号双向比对，把头文件的 `typedef struct` 块与载体报告的记录布局对照，编译一个对每条声明都持有类型化函数指针的 C99 探针，并编译一个 C++17 包含探针 —— 它包含 `spoke_connect.h` 与 `spoke_connect.hpp`（各两次，覆盖重复包含），并在禁用异常与 RTTI 的配置下实例化便利层的值、全部句柄与三个回调工厂。

由于记录布局报告来自载体自身的测试目标，该漂移门需要 `cargo` 位于 `PATH`；布局检查中"编译并断言"的那一半即该目标的定向运行 `cargo test -p spoke-connect-capi --lib abi_layout`，由漂移门代为调用。

```sh
node tooling/connect/cpp-symbol-check.mjs \
  --header crates/spoke-connect/bindings/cpp/include/spoke_connect.h \
  --library crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib
```

```text
C ABI symbols: PASS (83 declarations, 83 exports, 0 missing, 0 extra)
Record layout: 11 records, 66 assertions (sizeof/_Alignof/offsetof) match the carrier mirrors
C probe (clang -std=c99 -Wall -Wextra -Werror): PASS
C++17 inclusion (no exceptions, no RTTI; .h + .hpp): PASS
```

加上 `--self-test` 后，同一个入口点会在整个头文件目录的临时副本上运行反向对照：缺少真实声明的头文件、声明了载体未导出符号的头文件、被改动的回调签名，以及携带异常语法的 `.hpp` 变体 —— 最后一项证明该包含探针确实编译了便利头文件，而不是退化成只有 C 的检查。

## 下一步

- [把原生绑定接到 RemoteAdapter](/zh/how-to/remote-adapter-native-binding) —— C ABI 所携带的 adapter、路由器、响应方、ports 与工具面。
- [connect 线上参考](/zh/reference/connect) —— 信封字段表与身份绑定。
- [开启你的首个 connect 会话](/zh/tutorials/first-connect-session) —— 每个绑定都实现的握手流程。
