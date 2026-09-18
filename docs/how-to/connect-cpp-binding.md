---
title: Connect from C and C++
---

# Connect from C and C++

The C/C++ channel ships two hand-written headers and one committed carrier per platform. `spoke_connect.h` is the C contract — C99, with status values, value types, callback tables and every exported function, and ABI revision `1` reported through `spoke_connect_abi_version`. `spoke_connect.hpp` is a header-only C++17 convenience layer over that contract: namespace `spoke::connect`, move-only ownership for every owned buffer and handle, borrowed text views with explicit copies, one structured `Result` error channel, and host callback bridges. A C++17 host includes `spoke_connect.hpp` — which includes the C header — and links the native carrier.

The committed C/C++ carriers target macOS arm64 (`osx-arm64`) and Windows x64 (`win-x64`). Take `spoke_connect.h`, `spoke_connect.hpp`, and the native files for your target from the same repository tag `vX.Y.Z`.

The consumer default builds with exceptions and RTTI disabled: clang `-std=c++17 -fno-exceptions -fno-rtti`, MSVC `/std:c++17 /EHs-c- /GR- /MD /D_HAS_EXCEPTIONS=0`. Exception-enabled builds use the same error API — clang `-std=c++17 -fexceptions -fno-rtti`, MSVC `/std:c++17 /EHsc /GR- /MD` — and every translation unit that includes the convenience header in one linked image must use a consistent exception and standard-library configuration.

The carrier wraps the same public Rust facade the generated **native bindings** expose — the session core (`peer_id` derivation, hello sign/verify, allowlist, nonce store, sequence counters, response correlation, dispatch gate), `RemoteAdapter`, `MultiPeerRouter` and `ConnectResponder` with the `PortsHandler` and `ToolHandler` callbacks, and the in-memory loopback helpers. The parity table [`bindings/cpp/parity.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/parity.md) maps every production facade member, callback and error variant to its C declaration and records the C++ counterpart that consumes it.

## 1. Take the headers and the native

| Path | Contents |
|------|----------|
| `crates/spoke-connect/bindings/cpp/include/spoke_connect.h` | The C contract: status values, value types, callback tables and every exported function |
| `crates/spoke-connect/bindings/cpp/include/spoke_connect.hpp` | The C++17 convenience layer: `Buffer`, `Error`, `Result<T>`, the handle classes, the three callback records and their factories |
| `crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib` | macOS arm64 carrier, install name `@rpath/libspoke_connect_capi.dylib` |
| `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll` | Windows x64 carrier |
| `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll.lib` | The Rust-produced import library the Windows link step consumes |
| `crates/spoke-connect/bindings/cpp/native/provenance.json` | Per RID: source revision, target, `rustc -Vv`, compiler version, build flags, header and artifact hashes |
| `crates/spoke-connect/bindings/cpp/README.md` | The binding README with the full calling contract |

The C header compiles as C99 (fixed-width integers from `<stdint.h>`, lengths from `<stddef.h>`) and uses `extern "C"` inclusion for C++. Windows functions and callback pointers use `__cdecl`; macOS uses the platform default C calling convention. Every public symbol is prefixed `spoke_connect_`. The convenience header needs only the C++17 standard library and defines no configuration macro of its own.

## 2. Compile and link

macOS arm64 links the dylib and points the loader at its directory:

```sh
clang++ -std=c++17 -fno-exceptions -fno-rtti \
  -I crates/spoke-connect/bindings/cpp/include \
  host.cpp crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib \
  -Wl,-rpath,<absolute path to native/osx-arm64> \
  -o host
```

Windows x64 links the import library, pairs the release C++ CRT (`/MD`) with the carrier's Rust dynamic CRT, and loads the DLL from the executable's directory:

```bat
cl.exe /nologo /std:c++17 /EHs-c- /GR- /MD /D_HAS_EXCEPTIONS=0 /W4 /WX ^
  /I<absolute path to include> ^
  host.cpp <absolute path to native\win-x64\spoke_connect_capi.dll.lib> /Fe:host.exe
copy /Y <absolute path to native\win-x64\spoke_connect_capi.dll> .
host.exe
```

The validated Windows pairing is `/MD`: the release C++ CRT matches the carrier's Rust dynamic CRT. `/D_HAS_EXCEPTIONS=0` is the standard-library half of the no-exception configuration; an exception-enabled build replaces `/EHs-c- /D_HAS_EXCEPTIONS=0` with `/EHsc`, and the macOS flags with `-fexceptions -fno-rtti`.

## 3. Open a session

The three blocks below compose one complete `host.cpp`: a host-owned queue transport, the demo identity with a served echo tool, and the dial with one real invoke. Append them in order — every include and helper they use is shown.

### Block 1 — the host transport

```cpp
// ── host.cpp — append part 1 of 3 ───────────────────────────────────────────
//
// The host transport: the includes, the two `Result` helpers, and the queue
// pair whose callbacks carry every envelope this session sends and receives.

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
 * Returns the value of a successful construction step; a failure prints the
 * error and ends the process with its status. Only the steps before a session
 * exists use this — a post-establishment failure takes the ordered close path
 * in part 3 instead.
 */
template <typename T>
T unwrap(Result<T>&& result, const char* step) {
    if (result.has_value()) return std::move(result).value();
    std::cerr << step << ": status " << result.error().status << " — " << result.error().message
              << "\n";
    std::exit(result.error().status);
}

/** The `Result<void>` flavour of `unwrap`, for a step that reports no value. */
void unwrap(Result<void>&& result, const char* step) {
    if (result.has_value()) return;
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

### Block 2 — identity and server

```cpp
// ── host.cpp — append part 2 of 3 ───────────────────────────────────────────
//
// The demo identity, the capability manifest both ends advertise, and the
// serving end: both transports, `ConnectResponder::serve`, and the echo tool.

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
 * Creates both ends over the cross-wired queue pair, starts serving and
 * registers the echo tool. `ports` is absent — the service is offered no ports
 * callbacks at all, which is different from a provider that declines a method.
 */
Endpoints start_endpoints(const HostQueues& server_queues, const HostQueues& dialer_queues) {
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

    std::unique_ptr<connect::ToolCallbacks> echo_callbacks = make_echo_tool();
    connect::ToolHandler echo =
        unwrap(connect::ToolHandler::create(echo_callbacks), "ToolHandler::create");
    unwrap(responder.register_tool_handler(kEchoToolId, echo),
           "ConnectResponder::register_tool_handler");

    std::unique_ptr<connect::TransportCallbacks> dialer_callbacks =
        make_transport(dialer_queues);
    connect::Transport dialer_transport =
        unwrap(connect::Transport::create(dialer_callbacks), "Transport::create (dialer)");

    return Endpoints{std::move(server_transport), std::move(dialer_transport),
                     std::move(responder), std::move(echo), std::move(peer_id_text)};
}
```

### Block 3 — dial, one call, close

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

    Endpoints endpoints = start_endpoints(server_queues, dialer_queues);
    const SpokeConnectSlice seed = connect::bytes(kDemoSeed.data(), kDemoSeed.size());
    const SpokeConnectSlice pubkey = connect::bytes(kDemoPubkey.data(), kDemoPubkey.size());
    const SpokeConnectSlice allowlist[] = {connect::slice(endpoints.peer_id)};
    const std::string manifest = kManifest;

    // `connect` performs the signed hello and the handshake itself — no second
    // hello is sent by hand. Both ends share this example's identity, so the
    // dialed peer is this host.
    connect::RemoteAdapter adapter = unwrap(
        connect::RemoteAdapter::connect(endpoints.dialer_transport, seed, manifest, pubkey,
                                        allowlist, 1, uint64_t{5000}),
        "RemoteAdapter::connect");

    int status = SPOKE_CONNECT_OK;

    // A present session id is what an established session carries; the state
    // names the transition the dialer reached.
    Buffer state = unwrap(adapter.state(), "RemoteAdapter::state");
    Result<std::optional<Buffer>> session = adapter.session_id();
    const bool established = state.view() == "Established" && session.has_value() &&
                             session.value().has_value() && !session.value()->empty();
    if (established) {
        std::cout << "state: " << state.view()
                  << ", session id: " << session.value()->view() << "\n";
    } else {
        std::cerr << "the session did not establish (state \"" << state.view() << "\")\n";
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

    // Ordered close, on the failure path too: end both sessions explicitly, then
    // close the host queues so a blocked `recv` returns, then let this scope
    // release the RAII handles and every buffer it still holds.
    Result<void> dialer_closed = adapter.close();
    if (!dialer_closed.has_value()) {
        status = report(status, "RemoteAdapter::close", dialer_closed.error());
    }
    Result<void> responder_closed = endpoints.responder.close();
    if (!responder_closed.has_value()) {
        status = report(status, "ConnectResponder::close", responder_closed.error());
    }
    server_queues.close();
    dialer_queues.close();

    return status;
}
```

A green run of this program prints:

```text
state: Established, session id: connect-responder-session-12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf
tools.example.echo -> {"message":"hello"}
```

### What the values guarantee

| Concern | Contract |
|---------|----------|
| Borrowed text | `Buffer::view()` borrows the owned record — it stays valid while that `Buffer` is alive, it keeps the binary length (embedded NUL bytes are content), and it is deleted on a temporary buffer, whose bytes die with it. An empty buffer reads as an empty view. |
| Owned text | `Buffer::str()` materializes an owned `std::string`; use it when the value must outlive the buffer, as the peer id above does. |
| Results | Every fallible call returns `[[nodiscard]] Result<T>` or `Result<void>` carrying the original `SPOKE_CONNECT_*` status and the full structured `Error` (`message`, optional `code` / `kind` / `wire_code`, `expected`, `actual`). A non-zero C status always returns by value. |
| Bare values | Handles and buffers are move-only: a destructor releases, a moved-from object is empty, and `get()` / `release()` / `adopt()` exist for raw-C interop. |
| Session end | Destructors free only — they never close. `RemoteAdapter::close()` and `ConnectResponder::close()` end a session explicitly, and the host's own transport close releases the queues; close first, then let the scope release the handles. |
| Exceptions | The layer contains an escaping host-callback exception in exception-enabled builds: a transport callback becomes `SPOKE_CONNECT_TRANSPORT_IO`, a ports or tool callback becomes `SPOKE_CONNECT_FFI_REJECTED` with `code="INTERNAL_ERROR"`. With exceptions disabled there is no `try`, `catch` or `throw` in the header at all, and a callback reports failure through its `Result`. |
| Real networks | The three transport callbacks are the only transport seam: replace the queue pair with the host's network transport — one envelope per send, a blocking receive per `recv`, and a `close` that unblocks a pending receive — and everything above stays unchanged. The exported loopback helpers are a separate in-process demonstration and are never wrapped as a callback transport. |

## 4. Call the raw C ABI

The convenience layer is optional: `spoke_connect.h` is the contract, and a C host uses it directly.

| Concern | Contract |
|---------|----------|
| Status | Every call returns `int32_t`: `SPOKE_CONNECT_OK` (0) on success, or a `SPOKE_CONNECT_*` value with the detail in the caller's error record. Failed calls preserve caller ownership of the out values. |
| Out parameters | Zero-initialize each out value and the `SpokeConnectError` record before the call; the library fills the fields the result carries. |
| Owned results | Release `SpokeConnectBuffer` with `spoke_connect_buffer_free`, `SpokeConnectOptionalBuffer` with `spoke_connect_optional_buffer_free`, and `SpokeConnectError` with `spoke_connect_error_free`. |
| Borrowed inputs | `SpokeConnectSlice` borrows bytes for the duration of the call; the length is authoritative, text is validated UTF-8, and keys are 32 raw bytes. |
| Handles | Each handle owns one carrier object and releases with `<object>_free` (releasing a null handle is idempotent); `close` ends a session and leaves the release to `<object>_free`. Constructors and the router borrow the handles they are given. |
| Callback tables | `spoke_connect_transport_new`, `spoke_connect_ports_handler_new` and `spoke_connect_tool_handler_new` copy the table and take ownership of `user_data` on success, running its `destroy` once after the last reference and in-flight callback. |
| Returned buffers | A callback hands ownership of a populated `SpokeConnectForeignBuffer` to the carrier, which copies the bytes and calls the buffer's `release` exactly once. |
| Threading | Calls block the calling host thread; callbacks run on the carrier's blocking pool and may run concurrently, so host contexts are thread-safe. |
| Lifetime | Load the carrier for the process lifetime, and close sessions and release handles before host shutdown. |

Status values group the failure vocabulary:

| Range | Meaning |
|-------|---------|
| 0 | `SPOKE_CONNECT_OK` |
| 1 / 2 | `SPOKE_CONNECT_INVALID_ARGUMENT` (C-boundary validation) / `SPOKE_CONNECT_PANIC` (a contained panic) |
| 100–107 | Core errors: invalid hello signature, nonce replay, handshake failed, invalid nonce, crypto, JCS, token invalid, protocol version mismatch |
| 200–202 | Core invoke errors: sequence exhausted, inbound sequence mismatch, correlation mismatch |
| 300 / 301 | Dial error / rejected (application and dispatch refusals, with `code`, `kind` and `wire_code` preserved where the mapping defines them) |
| 400 / 401 | Transport closed / transport I/O |

## 5. Run the golden-parity smoke

`tooling/connect/cpp-smoke.mjs` compiles the committed `Smoke/main.cpp` (raw C ABI) and `Smoke/convenience.cpp` (convenience layer) translation units, links the staged native for the requested RID, and runs them against the shared golden vector `crates/spoke-connect/tests/fixtures/golden-hello.json`:

```sh
node tooling/connect/cpp-smoke.mjs --rid osx-arm64
node tooling/connect/cpp-smoke.mjs --rid win-x64
```

Within one RID the runner builds two configurations — exceptions disabled (the consumer default) and exceptions enabled — and requires each configuration's banner lines to equal its expected list exactly: same count, same order, same text. A green run prints, in order:

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

The exceptions-enabled configuration additionally prints `C++ callback exception containment: PASS` before the final line, because only that build injects a throwing host callback and observes the containment row. The runner ends with `C++ smoke runner: PASS (rid <rid>)`, and a missing, extra, repeated or reordered banner fails the run.

The Windows lane [`.github/workflows/cpp-connect.yml`](https://github.com/42ch-dev/spoke/blob/main/.github/workflows/cpp-connect.yml) compiles, links and runs the same smoke with MSVC on every pull request and main push that touches the carrier, its header, the smoke or the build/check/smoke scripts.

## 6. Consume the carrier from Unreal Engine

The Unreal Engine module reference ([`bindings/cpp/ue/SpokeConnect.Build.cs`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/ue/SpokeConnect.Build.cs) and its [README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/ue/README.md)) declares an external module that puts the header directory on the include path and links and stages the committed carrier for the target platform. UnrealBuildTool discovers modules in a project's or plugin's `Source/` tree, so vendor the `cpp/` tree as a unit — for example `MyProject/Source/SpokeConnect/`, containing `ue/SpokeConnect.Build.cs` beside `include/` and `native/` — and add the module to the consuming module's dependencies:

```csharp
PublicDependencyModuleNames.AddRange(new string[] { "SpokeConnect" });
```

| Target | Link | Runtime staging |
|--------|------|-----------------|
| `Win64` (x86_64) | `native/win-x64/spoke_connect_capi.dll.lib` through `PublicAdditionalLibraries` | `RuntimeDependencies.Add("$(TargetOutputDir)/spoke_connect_capi.dll", …)` copies the DLL beside the executable |
| `Mac` (arm64) | `native/osx-arm64/libspoke_connect_capi.dylib` through `PublicAdditionalLibraries` | `RuntimeDependencies.Add(…, StagedFileType.NonUFS)` keeps the dylib loose beside the executable |

The module accepts `Win64` with `x86_64` and `Mac` with `arm64`, reads the target architecture through the `UnrealArch` API (UE 5.2 onward), and contributes include paths plus link/staging wiring — the consuming module calls the C ABI functions it needs, owns the transport implementation, and decides which engine thread consumes callback results. A carrier call blocks the calling OS thread, so call the ABI from worker threads; callbacks arrive on the carrier's blocking pool and may run concurrently. Keep the carrier loaded for the process lifetime, close sessions, then release handles before host shutdown.

Validation status: the executed evidence is the standalone C++17 smoke for `osx-arm64` and `win-x64` (step 5); the module README lists the engine-side checks — editor build and load, packaged loose-library load, one connected session, orderly shutdown, target settings — as the maintainer checklist a machine with an engine environment runs.

## 7. Keep the headers and the exports in step

`tooling/connect/cpp-symbol-check.mjs` is the executable drift gate: it parses the header's declaration block, compares it against the carrier's exported `spoke_connect_*` symbols in both directions, checks the header's `typedef struct` block against the carrier's reported record layouts, compiles a C99 probe holding a typed function pointer to every declaration, and compiles a C++17 inclusion probe that includes `spoke_connect.h` and `spoke_connect.hpp` — twice, covering repeated inclusion — and instantiates the convenience layer's values, its full handle set and its three callback factories with exceptions and RTTI disabled.

Because the record-layout report comes from the carrier's own test target, the gate needs `cargo` on `PATH`; the compile-and-assert half of the layout pass is that target's scoped run, `cargo test -p spoke-connect-capi --lib abi_layout`, which the gate invokes for you.

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

With `--self-test`, the same entry point runs the negative controls in temporary copies of the whole header directory: a header missing a real declaration, one declaring a symbol the carrier does not export, a mutated callback signature, and a `.hpp` mutation carrying exception syntax — the last proves the inclusion probe really compiles the convenience header instead of degrading to a C-only check.

## Next steps

- [Bridge a native binding to RemoteAdapter](/how-to/remote-adapter-native-binding) — the adapter, router, responder, ports and tool surface the C ABI carries.
- [Connect wire reference](/reference/connect) — envelope field tables and identity binding.
- [Open your first connect session](/tutorials/first-connect-session) — the handshake flow every binding implements.
