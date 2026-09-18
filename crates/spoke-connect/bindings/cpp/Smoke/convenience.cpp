/*
 * C++17 convenience-layer smoke: the value and ownership layer, the core free
 * functions, the three core session objects, the loopback pair and ends, and —
 * over a host queue transport — the callback bridges with the adapter and
 * responder wrappers.
 *
 * The unit is linked with `Smoke/main.cpp` into one smoke program: both
 * translation units include the convenience header (through `support.hpp`), so
 * the build also covers single-header ODR. `main.cpp` reads the shared golden
 * vector once and hands it over — this unit never re-reads or re-transcribes it.
 *
 * The runner is `tooling/connect/cpp-smoke.mjs`, which builds this unit twice:
 * with exceptions disabled (the consumer default) and with exceptions enabled.
 * The exception-containment assertions exist only in the second configuration;
 * the first exercises the equivalent `Result` refusal path instead.
 */

#include "support.hpp"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <deque>
#include <memory>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <thread>
#include <utility>
#include <vector>

/**
 * Whether this translation unit is built with exception handling. The same
 * compiler facts the convenience header selects its containment branch from:
 * the throwing-callback proof exists only where the header contains it, and the
 * disabled configuration drives the equivalent `Result` refusal instead.
 */
#if defined(_CPPUNWIND) || defined(__cpp_exceptions) || defined(__EXCEPTIONS)
#define SPOKE_SMOKE_EXCEPTIONS 1
#else
#define SPOKE_SMOKE_EXCEPTIONS 0
#endif

namespace {

using spoke::connect::Buffer;
using spoke::connect::ConnectResponder;
using spoke::connect::Error;
using spoke::connect::PortsCallbacks;
using spoke::connect::PortsHandler;
using spoke::connect::RemoteAdapter;
using spoke::connect::Result;
using spoke::connect::ToolCallbacks;
using spoke::connect::ToolHandler;
using spoke::connect::Transport;
using spoke::connect::TransportCallbacks;
using spoke_smoke::banner;
using spoke_smoke::check;
using spoke_smoke::fail;
using spoke_smoke::Golden;
using spoke_smoke::json_string_field;

const char* const kValuesLabel = "C++ convenience values/core";

/** Renders an error record for a diagnostic message. */
std::string describe(const Error& error) {
    std::string text = "status " + std::to_string(error.status) + " message=\"" + error.message + "\"";
    if (error.code) text += " code=\"" + *error.code + "\"";
    if (error.kind) text += " kind=\"" + *error.kind + "\"";
    if (error.wire_code) text += " wire_code=\"" + *error.wire_code + "\"";
    text += " expected=" + std::to_string(error.expected) +
            " actual=" + std::to_string(error.actual);
    return text;
}

/** Fails unless the call succeeded, then returns the value it carried. */
template <typename T>
T unwrap(Result<T>&& result, const char* label, const std::string& detail) {
    if (!result.has_value()) fail(label, detail + " failed: " + describe(result.error()));
    return std::move(result).value();
}

/** The values/core group's flavour: failures report against its banner. */
template <typename T>
T unwrap(Result<T>&& result, const std::string& detail) {
    return unwrap(std::move(result), kValuesLabel, detail);
}

/** The `Result<void>` flavour of `unwrap`. */
void unwrap_ok(Result<void>&& result, const char* label, const std::string& detail) {
    if (!result.has_value()) fail(label, detail + " failed: " + describe(result.error()));
}

void unwrap_ok(Result<void>&& result, const std::string& detail) {
    unwrap_ok(std::move(result), kValuesLabel, detail);
}

/** Fails unless the call reported exactly `expected_status`. */
template <typename T>
void require_status(Result<T>& result, const char* label, int32_t expected_status,
                    const std::string& detail) {
    if (result.has_value()) fail(label, detail + " unexpectedly succeeded");
    const Error& error = result.error();
    if (error.status != expected_status) fail(label, detail + " reported " + describe(error));
}

/** The values/core group's flavour of `require_status`. */
template <typename T>
void require_status(Result<T>& result, int32_t expected_status, const std::string& detail) {
    require_status(result, kValuesLabel, expected_status, detail);
}

/** Fails unless the call reported a failure, then returns the `Error` it
    carried, so the caller can assert the fields it preserved. */
template <typename T>
const Error& rejection(Result<T>& result, const char* label, const std::string& detail) {
    if (result.has_value()) fail(label, detail + " unexpectedly succeeded");
    return result.error();
}

/** Borrows key material from the golden vector into the C slice form. */
SpokeConnectSlice slice_of(const std::vector<uint8_t>& bytes) {
    return spoke::connect::bytes(bytes.data(), bytes.size());
}

/** Polls `predicate` until it holds or the timeout elapses. */
template <typename Predicate>
bool wait_for(Predicate predicate, std::chrono::milliseconds timeout) {
    const std::chrono::steady_clock::time_point deadline =
        std::chrono::steady_clock::now() + timeout;
    while (std::chrono::steady_clock::now() < deadline) {
        if (predicate()) return true;
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
    return predicate();
}

/** The golden identity, the signed hello and the two version numbers. */
void assert_identity_and_versions(const Golden& golden) {
    using namespace spoke::connect;

    Buffer peer_id = unwrap(derive_peer_id_from_ed25519_pubkey(slice_of(golden.pubkey)),
                            "derive_peer_id_from_ed25519_pubkey");
    check(peer_id.view() == golden.peer_id, kValuesLabel,
          "the derived peer id \"" + std::string(peer_id.view()) +
              "\" differs from the golden vector \"" + golden.peer_id + "\"");

    Buffer hello = unwrap(sign_hello_ed25519(slice_of(golden.seed), golden.nonce,
                                             golden.manifest_json),
                          "sign_hello_ed25519");
    // An owned copy: the borrowed view dies with the buffer.
    const std::string hello_json = hello.str();
    check(json_string_field(hello_json, "signature") == golden.signature_b64u, kValuesLabel,
          "the signed hello carries signature \"" +
              json_string_field(hello_json, "signature") +
              "\" instead of the pinned golden signature \"" + golden.signature_b64u + "\"");
    check(json_string_field(hello_json, "peer_id") == golden.peer_id, kValuesLabel,
          "the signed hello carries peer id \"" + json_string_field(hello_json, "peer_id") + "\"");
    unwrap_ok(verify_hello_ed25519(slice_of(golden.pubkey), golden.peer_id, hello.view()),
              "verify_hello_ed25519");

    const uint64_t protocol = unwrap(protocol_version(), "protocol_version");
    check(protocol == 1, kValuesLabel,
          "protocol_version() reported " + std::to_string(protocol) + " instead of 1");
    const uint64_t abi = unwrap(abi_version(), "abi_version");
    check(abi == 1, kValuesLabel,
          "abi_version() reported " + std::to_string(abi) + " instead of the ABI revision 1");
}

/** A tampered hello is rejected with the hello status on the C++ error. */
void assert_tampered_hello(const Golden& golden) {
    using namespace spoke::connect;

    Buffer hello = unwrap(sign_hello_ed25519(slice_of(golden.seed), golden.nonce,
                                             golden.manifest_json),
                          "sign_hello_ed25519");
    std::string tampered = hello.str();
    const std::string role = "\"data-store\"";
    const size_t role_at = tampered.find(role);
    check(role_at != std::string::npos, kValuesLabel,
          "the signed golden hello carries no role field to tamper with");
    tampered.replace(role_at, role.size(), "\"checker\"");

    Result<void> verified =
        verify_hello_ed25519(slice_of(golden.pubkey), golden.peer_id, tampered);
    require_status(verified, SPOKE_CONNECT_INVALID_HELLO_SIGNATURE, "a tampered hello");
    check(!verified.error().message.empty(), kValuesLabel,
          "a tampered hello carried no message");
}

/** The identity, capability and correlation gates. */
void assert_gates(const Golden& golden) {
    using namespace spoke::connect;

    const std::string other_peer_id = "12D3KooWNotTheGoldenPeerId000000000000000000000";
    const SpokeConnectSlice allowlist[] = {slice(golden.peer_id)};
    check(unwrap(is_allowlisted(allowlist, 1, golden.peer_id), "is_allowlisted"), kValuesLabel,
          "the golden peer id is not on its own allowlist");
    check(!unwrap(is_allowlisted(allowlist, 1, other_peer_id), "is_allowlisted"), kValuesLabel,
          "an unknown peer id passed the allowlist");
    check(!unwrap(is_allowlisted(nullptr, 0, golden.peer_id), "is_allowlisted (empty)"),
          kValuesLabel, "an empty allowlist authorized a peer");

    const SpokeConnectSlice baseline[] = {slice("spoke-baseline")};
    check(unwrap(dispatch_allowed("check", baseline, 1), "dispatch_allowed"), kValuesLabel,
          "\"check\" was not authorized with the negotiated spoke-baseline capability");
    check(!unwrap(dispatch_allowed("check", nullptr, 0), "dispatch_allowed (no capabilities)"),
          kValuesLabel, "\"check\" was authorized with no negotiated capability");
    check(!unwrap(dispatch_allowed("custom-op", baseline, 1),
                  "dispatch_allowed (product-defined op)"),
          kValuesLabel, "a product-defined op was authorized by the core gate");

    std::optional<Buffer> capability = unwrap(required_capability("check"), "required_capability");
    if (!capability.has_value()) fail(kValuesLabel, "\"check\" reported no required capability");
    check(capability->view() == "spoke-baseline", kValuesLabel,
          "\"check\" requires \"" + std::string(capability->view()) + "\"");
    std::optional<Buffer> tool_capability =
        unwrap(required_capability("tools.example.echo"), "required_capability");
    if (!tool_capability.has_value()) {
        fail(kValuesLabel, "a self-describing tool reported no required capability");
    }
    check(tool_capability->view() == "tools.example.echo", kValuesLabel,
          "a self-describing tool requires \"" + std::string(tool_capability->view()) + "\"");
    std::optional<Buffer> product_capability =
        unwrap(required_capability("custom-op"), "required_capability");
    check(!product_capability.has_value(), kValuesLabel,
          "a product-defined op reported a required capability instead of an absent one");

    unwrap_ok(check_response_correlation("session-1", 7, "request-1", "session-1", 7,
                                         "request-1"),
              "check_response_correlation");
    Result<void> mismatch =
        check_response_correlation("session-1", 7, "request-1", "session-1", 8, "request-1");
    require_status(mismatch, SPOKE_CONNECT_CORRELATION_MISMATCH,
                   "a response with a different sequence");
    check(!mismatch.error().message.empty(), kValuesLabel,
          "a correlation mismatch carried no message");
}

/** The nonce store and the two sequence counters. */
void assert_core_objects(const Golden& golden) {
    using namespace spoke::connect;

    NonceStore store = unwrap(NonceStore::create(), "NonceStore::create");
    check(unwrap(store.check_and_record(golden.peer_id, golden.nonce),
                 "NonceStore::check_and_record"),
          kValuesLabel, "the nonce store did not record a fresh pair");
    check(!unwrap(store.check_and_record(golden.peer_id, golden.nonce),
                  "NonceStore::check_and_record (replay)"),
          kValuesLabel, "the nonce store accepted the same pair twice");

    OutboundSequence outbound = unwrap(OutboundSequence::create(), "OutboundSequence::create");
    const uint64_t first = unwrap(outbound.allocate(), "OutboundSequence::allocate");
    check(first == 0, kValuesLabel,
          "the first outbound allocation is " + std::to_string(first) + " instead of 0");
    check(unwrap(outbound.allocate(), "OutboundSequence::allocate") == 1, kValuesLabel,
          "the second outbound allocation did not advance the counter");

    InboundSequence inbound = unwrap(InboundSequence::create(), "InboundSequence::create");
    check(unwrap(inbound.advance(0), "InboundSequence::advance") == 1, kValuesLabel,
          "advancing over the first sequence did not expect 1 next");

    Result<uint64_t> replayed = inbound.advance(0);
    require_status(replayed, SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH,
                   "a replayed inbound sequence");
    check(replayed.error().expected == 1 && replayed.error().actual == 0, kValuesLabel,
          "the mismatch reported the wrong numbers: " + describe(replayed.error()));
    check(!replayed.error().message.empty(), kValuesLabel,
          "the sequence mismatch carried no message");

    check(unwrap(inbound.advance(1), "InboundSequence::advance") == 2, kValuesLabel,
          "the counter did not advance after rejecting the replay");
}

/** Ownership: adopt/release, moves, empties and raw-handle interop. */
void assert_ownership_and_moves(const Golden& golden) {
    using namespace spoke::connect;

    // adopt()/release(): ownership leaves the wrapper and comes back, and the
    // carrier releases the record exactly once either way.
    Buffer owned = unwrap(derive_peer_id_from_ed25519_pubkey(slice_of(golden.pubkey)),
                          "derive_peer_id_from_ed25519_pubkey");
    const std::string peer_id_text = owned.str();
    const SpokeConnectBuffer released = owned.release();
    check(owned.data() == nullptr && owned.size() == 0 && owned.empty(), kValuesLabel,
          "a released buffer still reports the transferred record");
    check(released.data != nullptr && released.len == peer_id_text.size(), kValuesLabel,
          "release() did not transfer the record");
    Buffer readopted = Buffer::adopt(released);
    check(readopted.view() == peer_id_text, kValuesLabel, "the adopted record lost its payload");

    // Move construction empties the source; move assignment releases the
    // target's previous record before taking the new one.
    Buffer source = unwrap(derive_peer_id_from_ed25519_pubkey(slice_of(golden.pubkey)),
                           "derive_peer_id_from_ed25519_pubkey");
    Buffer moved = std::move(source);
    check(source.data() == nullptr && source.size() == 0, kValuesLabel,
          "a moved-from buffer is not empty");
    check(moved.str() == peer_id_text, kValuesLabel, "the moved-to buffer does not own the payload");

    std::optional<Buffer> capability = unwrap(required_capability("check"), "required_capability");
    if (!capability.has_value()) fail(kValuesLabel, "\"check\" reported no required capability");
    Buffer target = std::move(*capability);
    target = std::move(moved);
    check(target.str() == peer_id_text, kValuesLabel,
          "move assignment did not transfer the payload");

    // Handles: move-only, empty once moved from, and never dangling.
    NonceStore store = unwrap(NonceStore::create(), "NonceStore::create");
    NonceStore taken = std::move(store);
    check(store.get() == nullptr && !static_cast<bool>(store), kValuesLabel,
          "a moved-from handle is not empty");
    check(taken.get() != nullptr, kValuesLabel, "the moved-to handle owns nothing");
    check(unwrap(taken.check_and_record(golden.peer_id, golden.nonce),
                 "NonceStore::check_and_record"),
          kValuesLabel, "the moved-to nonce store did not record a fresh pair");
    Result<bool> dangling = store.check_and_record(golden.peer_id, "golden-nonce-000000000002");
    require_status(dangling, SPOKE_CONNECT_INVALID_ARGUMENT,
                   "a call on a moved-from handle");

    // release()/adopt(): the caller takes the raw handle and hands it back.
    LoopbackTransportPair pair =
        unwrap(LoopbackTransportPair::create(), "LoopbackTransportPair::create");
    SpokeConnectLoopbackTransportPair* raw = pair.release();
    check(pair.get() == nullptr && raw != nullptr, kValuesLabel,
          "release() did not transfer the pair handle");
    pair.adopt(raw);
    check(pair.get() == raw, kValuesLabel, "adopt() did not take the raw pair handle back");
    pair.adopt(nullptr);
    check(pair.get() == nullptr, kValuesLabel, "adopt(nullptr) left a handle owned");
}

/** The loopback pair and ends, including a blocking recv woken by close. */
void assert_loopback() {
    using namespace spoke::connect;

    LoopbackTransportPair pair =
        unwrap(LoopbackTransportPair::create(), "LoopbackTransportPair::create");
    LoopbackTransport client = unwrap(pair.client(), "LoopbackTransportPair::client");
    LoopbackTransport server = unwrap(pair.server(), "LoopbackTransportPair::server");

    const std::string to_server = "cpp-convenience-envelope-0001";
    const std::string to_client = "cpp-convenience-envelope-0002";
    unwrap_ok(client.send(to_server), "LoopbackTransport::send (client)");
    Buffer received_server = unwrap(server.recv(), "LoopbackTransport::recv (server)");
    check(received_server.view() == to_server, kValuesLabel,
          "the server end received different bytes than the client end sent");
    unwrap_ok(server.send(to_client), "LoopbackTransport::send (server)");
    Buffer received_client = unwrap(client.recv(), "LoopbackTransport::recv (client)");
    check(received_client.view() == to_client, kValuesLabel,
          "the client end received different bytes than the server end sent");

    // The connection is empty here, so a receive on it stays pending: probing
    // that before close is what makes the wake-up after close an observation
    // rather than a coincidence.
    std::atomic<bool> finished{false};
    std::atomic<int32_t> observed{SPOKE_CONNECT_OK};
    std::thread waiter([&server, &finished, &observed] {
        Result<Buffer> woken = server.recv();
        observed.store(woken.has_value() ? SPOKE_CONNECT_OK : woken.error().status);
        finished.store(true);
    });
    std::this_thread::sleep_for(std::chrono::milliseconds(200));
    check(!finished.load(), kValuesLabel, "recv returned before the connection closed");

    unwrap_ok(client.close(), "LoopbackTransport::close");
    check(wait_for([&finished] { return finished.load(); }, std::chrono::milliseconds(5000)),
          kValuesLabel, "close did not wake the blocked recv");
    waiter.join();
    check(observed.load() == SPOKE_CONNECT_TRANSPORT_CLOSED, kValuesLabel,
          "a recv woken by close reported status " + std::to_string(observed.load()));

    unwrap_ok(server.close(), "LoopbackTransport::close (idempotent)");
    Result<Buffer> closed_recv = server.recv();
    require_status(closed_recv, SPOKE_CONNECT_TRANSPORT_CLOSED, "a recv after close");
    check(!closed_recv.error().message.empty(), kValuesLabel,
          "a recv after close carried no message");
}

// ── Callback bridges and the session wrappers ────────────────────────────

const char* const kSessionLabel = "C++ convenience callbacks/session";
#if SPOKE_SMOKE_EXCEPTIONS
/** The exceptions-enabled configuration's additional banner. */
const char* const kContainmentLabel = "C++ callback exception containment";
#endif
const char* const kEchoToolId = "tools.example.echo";
const char* const kEntryIdText = "cpp-convenience-entry-0001";
const char* const kEntryCanonicalName = "C++ convenience knowledge entry";
const char* const kServedEntryJson =
    "{\"schema_version\":1,\"entry_id\":\"cpp-convenience-entry-0001\","
    "\"entry_type\":\"note\",\"canonical_name\":\"C++ convenience knowledge entry\","
    "\"status\":\"confirmed\",\"body\":{\"summary\":\"served through the convenience callback "
    "bridge\"},\"extensions\":{}}";
const char* const kRefusalMessage = "the smoke's host tool handler refuses";
const char* const kRefusalKind = "smoke_reject_kind";
const char* const kRefusalWireCode = "smoke_reject_wire";
const char* const kThrowMessage = "the smoke's host tool handler throws";
/** The arguments the echo tool is asked to echo. */
const char* const kEchoArguments = R"({"message":"hello"})";
/** The arguments that make the host handler refuse with every error field set. */
const char* const kRefusalArguments = R"({"refuse":"fields"})";
/** The arguments that make the host handler refuse with a present empty `kind`. */
const char* const kEmptyKindArguments = R"({"refuse":"empty-kind"})";
/** The arguments that make the host handler throw (exception-enabled builds). */
const char* const kThrowArguments = R"({"refuse":"throw"})";

/**
 * One direction of the cross-wired host queue pair: an unbounded FIFO with a
 * close flag. Both ends share the same two queues, and every callback that
 * touches one runs on the carrier's blocking pool, so the state is guarded and
 * a close wakes every waiter.
 */
class EnvelopeQueue {
  public:
    /** Enqueues one envelope and reports whether it was accepted: a push into a
        closed queue returns false, so a send that can no longer be delivered is
        reported to the carrier instead of being dropped silently. */
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

/** What the host observed about one callback record: the calls the carrier ran
    and the release of the record itself. */
struct RecordCounters {
    std::atomic<long> sends{0};
    std::atomic<long> recvs{0};
    std::atomic<long> calls{0};
    std::atomic<long> releases{0};
};

/**
 * One host transport record's shared state. Every callback of the record
 * captures a `shared_ptr` to it by value, so the object dies with the record the
 * carrier owns — and that destructor is the observable "the carrier released
 * this record exactly once". The counters stay owned by the caller, which reads
 * them after the record is gone.
 */
struct HostTransport {
    HostTransport(std::shared_ptr<EnvelopeQueue> outbound_queue,
                  std::shared_ptr<EnvelopeQueue> inbound_queue, RecordCounters* observed) noexcept
        : outbound(std::move(outbound_queue)),
          inbound(std::move(inbound_queue)),
          counters(observed) {}
    ~HostTransport() { counters->releases.fetch_add(1); }

    std::shared_ptr<EnvelopeQueue> outbound;
    std::shared_ptr<EnvelopeQueue> inbound;
    RecordCounters* counters;
};

/** One host ports record's shared state: the calls it served, the answers they
    saw, and the optional base revision of the last `putKnowledgeEntry`. */
struct HostPorts {
    explicit HostPorts(RecordCounters* observed) noexcept : counters(observed) {}
    ~HostPorts() { counters->releases.fetch_add(1); }

    RecordCounters* counters;
    std::mutex mutex;
    long calls = 0;
    std::string last_entry_id;
    std::optional<uint64_t> last_revision;
};

/** One host tool record's shared state. */
struct HostTool {
    explicit HostTool(RecordCounters* observed) noexcept : counters(observed) {}
    ~HostTool() { counters->releases.fetch_add(1); }

    RecordCounters* counters;
    std::mutex mutex;
    long calls = 0;
    std::string last_arguments;
};

/**
 * The golden host manifest with the demo echo tool added: the fixture's own
 * manifest text is the base, so the smoke never restates the golden host
 * identity or its baseline capability. Both ends advertise the result, so
 * `tools.example.echo` is in the negotiated capability set — which is what the
 * dispatch gate requires for a tool invoke in either direction.
 */
std::string tool_manifest(const Golden& golden) {
    std::string manifest = golden.manifest_json;
    const std::string tool = kEchoToolId;
    const std::string capabilities = "\"capabilities\":[\"spoke-baseline\"]";
    const std::string namespaces = "\"namespaces\":[]";
    if (manifest.find(capabilities) == std::string::npos ||
        manifest.find(namespaces) == std::string::npos) {
        fail(kSessionLabel, "the golden manifest no longer carries the baseline capability and an "
                            "empty namespace list to extend: " + manifest);
    }
    manifest.replace(manifest.find(capabilities), capabilities.size(),
                     "\"capabilities\":[\"spoke-baseline\",\"" + tool + "\"]");
    manifest.replace(manifest.find(namespaces), namespaces.size(),
                     "\"namespaces\":[\"example\"]");
    const std::string descriptor = "{\"capability_id\":\"" + tool +
                                   "\",\"description\":\"Echo the arguments\","
                                   "\"input\":{\"type\":\"object\"},\"op\":\"" +
                                   tool +
                                   "\",\"output\":{\"type\":\"object\"},\"schema_version\":1}";
    manifest.insert(manifest.rfind('}'), ",\"tools\":[" + descriptor + "]");
    return manifest;
}

/** The transport-closed failure a host reports for a queue that can no longer
    deliver or receive. */
Error host_closed(const std::string& message) {
    Error error;
    error.status = SPOKE_CONNECT_TRANSPORT_CLOSED;
    error.message = message;
    return error;
}

/** Builds one transport record over a queue pair. Every callback captures the
    shared host state by value, so nothing outlives the carrier's `destroy`, and
    no callback reaches back into an operational API. */
std::unique_ptr<TransportCallbacks> make_transport(std::shared_ptr<HostTransport> host) {
    auto callbacks = std::make_unique<TransportCallbacks>();
    callbacks->send = [host](std::string_view envelope) -> Result<void> {
        host->counters->sends.fetch_add(1);
        if (!host->outbound->push(std::string(envelope))) {
            return Result<void>::failure(host_closed("the host transport queue is closed"));
        }
        return Result<void>::success();
    };
    callbacks->recv = [host]() -> Result<std::string> {
        host->counters->recvs.fetch_add(1);
        std::string envelope;
        if (!host->inbound->pop(&envelope)) {
            return Result<std::string>::failure(host_closed("the host transport queue is closed"));
        }
        return Result<std::string>::success(std::move(envelope));
    };
    callbacks->close = [host]() -> Result<void> {
        host->outbound->close();
        host->inbound->close();
        return Result<void>::success();
    };
    return callbacks;
}

/** Builds the served ports record: the knowledge-entry pair answers the canned
    entry — `putKnowledgeEntry` also records the optional base revision it was
    given — and every other slot declines. A host that does not serve a method
    still implements it, so an explicitly refusing provider stays
    distinguishable from an absent `ports` handler. */
std::unique_ptr<PortsCallbacks> make_ports(std::shared_ptr<HostPorts> host) {
    auto callbacks = std::make_unique<PortsCallbacks>();
    callbacks->get_knowledge_entry = [host](std::string_view entry_id) -> Result<std::string> {
        host->counters->calls.fetch_add(1);
        {
            std::lock_guard<std::mutex> guard(host->mutex);
            host->calls += 1;
            host->last_entry_id = std::string(entry_id);
        }
        return Result<std::string>::success(std::string(kServedEntryJson));
    };
    callbacks->put_knowledge_entry = [host](std::string_view,
                                            std::optional<uint64_t> expected_base_revision)
        -> Result<std::string> {
        host->counters->calls.fetch_add(1);
        {
            std::lock_guard<std::mutex> guard(host->mutex);
            host->calls += 1;
            host->last_revision = expected_base_revision;
        }
        return Result<std::string>::success(std::string(kServedEntryJson));
    };
    const auto decline = [host]() -> Result<std::string> {
        host->counters->calls.fetch_add(1);
        Error error;
        error.status = SPOKE_CONNECT_FFI_REJECTED;
        error.code = "CAPABILITY_PORT_MISSING";
        error.message = "the smoke's host ports record does not serve that method";
        return Result<std::string>::failure(std::move(error));
    };
    const auto refusal = [decline](std::string_view) -> Result<std::string> { return decline(); };
    const auto revision_refusal = [decline](std::string_view,
                                            std::optional<uint64_t>) -> Result<std::string> {
        return decline();
    };
    callbacks->get_relation = refusal;
    callbacks->put_relation = revision_refusal;
    callbacks->list_knowledge_entries = refusal;
    callbacks->list_timeline_events = refusal;
    callbacks->put_findings = refusal;
    callbacks->list_rules = [decline](const SpokeConnectSlice*, size_t) -> Result<std::string> {
        return decline();
    };
    callbacks->list_peer_host_capability_manifests = [decline]() -> Result<std::string> {
        return decline();
    };
    callbacks->project = refusal;
    callbacks->compute = refusal;
    callbacks->list_fork_timeline_events = refusal;
    callbacks->extract = refusal;
    return callbacks;
}

/**
 * Builds the served echo tool record. It echoes the arguments, unless they carry
 * one of the refusal markers: a refusal with every error field set, a refusal
 * whose optional `kind` is present and empty, or — in an exception-enabled
 * build — a throw, which is where the disabled build returns the equivalent
 * `Result` failure instead.
 */
std::unique_ptr<ToolCallbacks> make_echo_tool(std::shared_ptr<HostTool> host) {
    auto callbacks = std::make_unique<ToolCallbacks>();
    callbacks->handle = [host](std::string_view arguments_json) -> Result<std::string> {
        const std::string arguments(arguments_json);
        host->counters->calls.fetch_add(1);
        {
            std::lock_guard<std::mutex> guard(host->mutex);
            host->calls += 1;
            host->last_arguments = arguments;
        }
        if (arguments.find(kRefusalArguments) != std::string::npos) {
            Error error;
            error.status = SPOKE_CONNECT_FFI_REJECTED;
            error.code = "INVALID_INPUT";
            error.message = kRefusalMessage;
            error.kind = kRefusalKind;
            error.wire_code = kRefusalWireCode;
            return Result<std::string>::failure(std::move(error));
        }
        if (arguments.find(kEmptyKindArguments) != std::string::npos) {
            Error error;
            error.status = SPOKE_CONNECT_FFI_REJECTED;
            error.code = "INVALID_INPUT";
            error.message = kRefusalMessage;
            // Present and empty: the bridge must not collapse it into absent.
            error.kind = std::string();
            return Result<std::string>::failure(std::move(error));
        }
        if (arguments.find(kThrowArguments) != std::string::npos) {
#if SPOKE_SMOKE_EXCEPTIONS
            throw std::runtime_error(kThrowMessage);
#else
            Error error;
            error.status = SPOKE_CONNECT_FFI_REJECTED;
            error.code = "INVALID_INPUT";
            error.message = kThrowMessage;
            return Result<std::string>::failure(std::move(error));
#endif
        }
        return Result<std::string>::success("{\"echo\":" + arguments + "}");
    };
    return callbacks;
}

/** Waits for one host record to be released exactly once, then reports it. */
template <typename Watch>
bool record_released(const RecordCounters& counters, const Watch& watch) {
    const bool counted = wait_for([&counters] { return counters.releases.load() >= 1; },
                                  std::chrono::milliseconds(5000));
    return counted && counters.releases.load() == 1 && watch.expired();
}

/** The callbacks/session proofs: a host queue transport under the adapter and
    responder wrappers, one real baseline port round trip, a tool round trip in
    both directions, the rejection rows and the ownership rules. */
void assert_session(const Golden& golden) {
    using namespace spoke::connect;

    // Host side: two cross-wired queues, the records built over them, and the
    // counters the assertions read once the records are gone.
    const auto to_responder = std::make_shared<EnvelopeQueue>();
    const auto to_dialer = std::make_shared<EnvelopeQueue>();
    RecordCounters dialer_counters;
    RecordCounters responder_counters;
    RecordCounters ports_counters;
    RecordCounters served_tool_counters;
    RecordCounters reverse_tool_counters;

    std::shared_ptr<HostTransport> dialer_host =
        std::make_shared<HostTransport>(to_responder, to_dialer, &dialer_counters);
    std::shared_ptr<HostTransport> responder_host =
        std::make_shared<HostTransport>(to_dialer, to_responder, &responder_counters);
    std::shared_ptr<HostPorts> ports_host = std::make_shared<HostPorts>(&ports_counters);
    std::shared_ptr<HostTool> served_tool_host =
        std::make_shared<HostTool>(&served_tool_counters);
    std::shared_ptr<HostTool> reverse_tool_host =
        std::make_shared<HostTool>(&reverse_tool_counters);
    const std::weak_ptr<HostTransport> dialer_released = dialer_host;
    const std::weak_ptr<HostTransport> responder_released = responder_host;
    const std::weak_ptr<HostPorts> ports_released = ports_host;
    const std::weak_ptr<HostTool> served_tool_released = served_tool_host;
    const std::weak_ptr<HostTool> reverse_tool_released = reverse_tool_host;

    // The golden fixture supplies the identity; the manifest derived from it
    // advertises the demo echo tool on both ends.
    const SpokeConnectSlice seed = slice_of(golden.seed);
    const SpokeConnectSlice pubkey = slice_of(golden.pubkey);
    const SpokeConnectSlice peer_id = slice(golden.peer_id);
    const SpokeConnectSlice allowlist[] = {peer_id};
    const SpokeConnectPeerKey peer_keys[] = {SpokeConnectPeerKey{peer_id, pubkey}};
    const std::string manifest = tool_manifest(golden);
    const std::optional<uint64_t> invoke_timeout = uint64_t{5000};

    // A factory refuses an incomplete record and leaves it with the caller,
    // which still owns the context the record captures.
    {
        RecordCounters refused_counters;
        std::shared_ptr<HostTransport> refused_host =
            std::make_shared<HostTransport>(to_responder, to_dialer, &refused_counters);
        const std::weak_ptr<HostTransport> refused_released = refused_host;
        std::unique_ptr<TransportCallbacks> incomplete = make_transport(refused_host);
        // Only the record's own captures keep the host state alive now.
        refused_host.reset();
        incomplete->close = nullptr;
        Result<Transport> refused = Transport::create(incomplete);
        const Error& error =
            rejection(refused, kSessionLabel, "Transport::create with an unset slot");
        check(error.status == SPOKE_CONNECT_INVALID_ARGUMENT, kSessionLabel,
              "an incomplete transport record reported " + describe(error));
        check(error.message.find("close") != std::string::npos, kSessionLabel,
              "the refusal does not name the unset slot: " + error.message);
        check(incomplete != nullptr, kSessionLabel,
              "a refused factory took the caller's callback record");
        check(!refused_released.expired() && refused_counters.releases.load() == 0, kSessionLabel,
              "a refused factory released the caller-owned context");
        incomplete.reset();
        check(record_released(refused_counters, refused_released), kSessionLabel,
              "the caller's refused callback record was not released exactly once");
    }
    {
        std::unique_ptr<PortsCallbacks> absent;
        Result<PortsHandler> refused = PortsHandler::create(absent);
        const Error& error =
            rejection(refused, kSessionLabel, "PortsHandler::create with no record");
        check(error.status == SPOKE_CONNECT_INVALID_ARGUMENT, kSessionLabel,
              "a missing ports record reported " + describe(error));
    }

    {
        // The serving side: its transport, its ports provider and its tool.
        std::unique_ptr<TransportCallbacks> responder_callbacks = make_transport(responder_host);
        std::unique_ptr<PortsCallbacks> ports_callbacks = make_ports(ports_host);
        std::unique_ptr<ToolCallbacks> served_tool_callbacks = make_echo_tool(served_tool_host);
        // Only the records' own captures keep the host state alive from here.
        responder_host.reset();
        ports_host.reset();
        served_tool_host.reset();

        Transport responder_transport =
            unwrap(Transport::create(responder_callbacks), kSessionLabel,
                   "Transport::create (responder)");
        check(responder_callbacks == nullptr, kSessionLabel,
              "a successful factory left the callback record owned by the caller");
        PortsHandler ports =
            unwrap(PortsHandler::create(ports_callbacks), kSessionLabel, "PortsHandler::create");
        check(ports_callbacks == nullptr, kSessionLabel,
              "a successful factory left the ports record owned by the caller");
        ToolHandler served_tool = unwrap(ToolHandler::create(served_tool_callbacks), kSessionLabel,
                                         "ToolHandler::create (served)");
        check(served_tool_callbacks == nullptr, kSessionLabel,
              "a successful factory left the tool record owned by the caller");

        ConnectResponder responder = unwrap(
            ConnectResponder::serve(responder_transport, seed, manifest, allowlist, 1, peer_keys,
                                    1, &ports, invoke_timeout),
            kSessionLabel, "ConnectResponder::serve");
        unwrap_ok(responder.register_tool_handler(kEchoToolId, served_tool), kSessionLabel,
                  "ConnectResponder::register_tool_handler");
        {
            // The carrier cloned the handler: releasing the wrapper must not
            // release a record the session still holds.
            ToolHandler released = std::move(served_tool);
            check(served_tool.get() == nullptr && !static_cast<bool>(served_tool), kSessionLabel,
                  "a moved-from handle is not empty");
        }

        // The dialing side.
        std::unique_ptr<TransportCallbacks> dialer_callbacks = make_transport(dialer_host);
        dialer_host.reset();
        Transport dialer_transport = unwrap(Transport::create(dialer_callbacks), kSessionLabel,
                                            "Transport::create (dialer)");
        check(dialer_callbacks == nullptr, kSessionLabel,
              "a successful factory left the dialer record owned by the caller");
        RemoteAdapter adapter = unwrap(
            RemoteAdapter::connect(dialer_transport, seed, manifest, pubkey, allowlist, 1,
                                   invoke_timeout),
            kSessionLabel, "RemoteAdapter::connect");

        // Both sessions cloned their transport internally, so releasing the
        // caller's transport handles must not release a callback record a
        // session still holds — and the session must keep working afterwards.
        {
            std::unique_ptr<Transport> released =
                std::make_unique<Transport>(std::move(responder_transport));
            released.reset();
            std::unique_ptr<Transport> released_dialer =
                std::make_unique<Transport>(std::move(dialer_transport));
            released_dialer.reset();
        }
        check(!dialer_released.expired() && !responder_released.expired(), kSessionLabel,
              "releasing a transport handle released a callback record the session still holds");
        check(dialer_counters.releases.load() == 0 && responder_counters.releases.load() == 0,
              kSessionLabel, "a transport callback record was destroyed while its session lived");

        // The session is established on both ends.
        const Buffer dialer_state =
            unwrap(adapter.state(), kSessionLabel, "RemoteAdapter::state");
        check(dialer_state.view() == "Established", kSessionLabel,
              "the dialer reported state \"" + std::string(dialer_state.view()) + "\"");
        const bool responder_established = wait_for(
            [&responder] {
                Result<Buffer> state = responder.state();
                return state.has_value() && state.value().view() == "Established";
            },
            std::chrono::milliseconds(5000));
        check(responder_established, kSessionLabel, "the responder did not reach Established");

        // The session carries the golden identity, and the manifest both ends
        // advertise crossed with it.
        std::optional<Buffer> session_id =
            unwrap(adapter.session_id(), kSessionLabel, "RemoteAdapter::session_id");
        check(session_id.has_value() && !session_id->empty(), kSessionLabel,
              "the established dialer reported no session id");
        std::optional<Buffer> remote_peer =
            unwrap(adapter.remote_peer_id(), kSessionLabel, "RemoteAdapter::remote_peer_id");
        check(remote_peer.has_value() && remote_peer->view() == golden.peer_id, kSessionLabel,
              "the dialer's remote peer id is not the golden identity");
        std::optional<Buffer> remote_manifest =
            unwrap(adapter.remote_manifest(), kSessionLabel, "RemoteAdapter::remote_manifest");
        check(remote_manifest.has_value() &&
                  remote_manifest->view().find(kEchoToolId) != std::string_view::npos,
              kSessionLabel, "the dialer's remote manifest does not advertise the served tool");

        // One real baseline ports round trip through the responder's provider.
        const std::string entry =
            unwrap(adapter.get_knowledge_entry(kEntryIdText), kSessionLabel,
                   "RemoteAdapter::get_knowledge_entry")
                .str();
        check(entry.find(std::string("\"entry_id\":\"") + kEntryIdText + "\"") !=
                  std::string::npos,
              kSessionLabel, "the round-tripped entry carries no entry id: " + entry);
        check(entry.find(kEntryCanonicalName) != std::string::npos, kSessionLabel,
              "the round-tripped entry carries no canonical name: " + entry);
        {
            std::shared_ptr<HostPorts> served = ports_released.lock();
            check(served != nullptr, kSessionLabel,
                  "the ports record was released while its session lived");
            std::lock_guard<std::mutex> guard(served->mutex);
            check(served->calls == 1, kSessionLabel,
                  "the served ports callback ran " + std::to_string(served->calls) + " times");
            check(served->last_entry_id == kEntryIdText, kSessionLabel,
                  "the served ports callback saw entry id \"" + served->last_entry_id + "\"");
        }

        // An absent expectation and a present zero are different values: the
        // optional scalar must not collapse one into the other.
        std::shared_ptr<HostPorts> ports_observer = ports_released.lock();
        check(ports_observer != nullptr, kSessionLabel,
              "the ports record was released while its session lived");
        const std::string written =
            unwrap(adapter.put_knowledge_entry(kServedEntryJson, std::nullopt), kSessionLabel,
                   "RemoteAdapter::put_knowledge_entry (no expectation)")
                .str();
        check(written.find(kEntryCanonicalName) != std::string::npos, kSessionLabel,
              "the round-tripped put carries no entry: " + written);
        {
            std::lock_guard<std::mutex> guard(ports_observer->mutex);
            check(!ports_observer->last_revision.has_value(), kSessionLabel,
                  "an absent base revision arrived as present");
        }
        unwrap(adapter.put_knowledge_entry(kServedEntryJson, uint64_t{0}), kSessionLabel,
               "RemoteAdapter::put_knowledge_entry (present zero)");
        {
            std::lock_guard<std::mutex> guard(ports_observer->mutex);
            check(ports_observer->last_revision.has_value() &&
                      *ports_observer->last_revision == 0,
                  kSessionLabel, "a present zero base revision did not arrive as a present zero");
        }
        ports_observer.reset();

        // A slot the host declines answers an application reject, not a silent
        // absence.
        Result<Buffer> declined = adapter.list_rules(nullptr, 0);
        const Error& refusal = rejection(declined, kSessionLabel, "a declining ports method");
        check(refusal.status == SPOKE_CONNECT_FFI_REJECTED && refusal.code.has_value() &&
                  *refusal.code == "CAPABILITY_PORT_MISSING",
              kSessionLabel, "a declining ports method reported " + describe(refusal));

        // The dialer invokes the responder's registered tool.
        const std::string echoed =
            unwrap(adapter.invoke_tool(kEchoToolId, kEchoArguments), kSessionLabel,
                   "RemoteAdapter::invoke_tool (dialer -> responder)")
                .str();
        check(echoed.find("\"message\":\"hello\"") != std::string::npos, kSessionLabel,
              "the round-tripped echo carries no echo of the arguments: " + echoed);
        {
            std::shared_ptr<HostTool> served = served_tool_released.lock();
            check(served != nullptr, kSessionLabel,
                  "the served tool record was released while its session lived");
            std::lock_guard<std::mutex> guard(served->mutex);
            check(served->calls == 1, kSessionLabel,
                  "the served tool handler ran " + std::to_string(served->calls) + " times");
            check(served->last_arguments == kEchoArguments, kSessionLabel,
                  "the served tool handler saw arguments \"" + served->last_arguments + "\"");
        }

        // The dialer registers its own handler and the responder invokes it in
        // reverse; releasing the wrapper must not release the cloned record.
        std::unique_ptr<ToolCallbacks> reverse_callbacks = make_echo_tool(reverse_tool_host);
        reverse_tool_host.reset();
        ToolHandler reverse_tool = unwrap(ToolHandler::create(reverse_callbacks), kSessionLabel,
                                         "ToolHandler::create (dialer)");
        unwrap_ok(adapter.register_tool_handler(kEchoToolId, reverse_tool), kSessionLabel,
                  "RemoteAdapter::register_tool_handler");
        {
            ToolHandler released = std::move(reverse_tool);
        }
        const std::string reversed =
            unwrap(responder.invoke_tool(kEchoToolId, kEchoArguments), kSessionLabel,
                   "ConnectResponder::invoke_tool (responder -> dialer)")
                .str();
        check(reversed.find("\"message\":\"hello\"") != std::string::npos, kSessionLabel,
              "the reverse invoke carries no echo of the arguments: " + reversed);
        {
            std::shared_ptr<HostTool> serving = reverse_tool_released.lock();
            check(serving != nullptr, kSessionLabel,
                  "the dialer's tool record was released while its session lived");
            std::lock_guard<std::mutex> guard(serving->mutex);
            check(serving->calls == 1, kSessionLabel,
                  "the dialer's registered handler served " + std::to_string(serving->calls) +
                      " reverse invokes");
            check(serving->last_arguments == kEchoArguments, kSessionLabel,
                  "the reverse invoke reached the handler with arguments \"" +
                      serving->last_arguments + "\"");
        }

        // A host refusal crosses the wire with all four error fields intact.
        Result<Buffer> refused_fields = adapter.invoke_tool(kEchoToolId, kRefusalArguments);
        const Error& fields = rejection(refused_fields, kSessionLabel, "a host tool-handler refusal");
        check(fields.status == SPOKE_CONNECT_FFI_REJECTED, kSessionLabel,
              "the refusal reported " + describe(fields));
        check(fields.code.has_value() && *fields.code == "INVALID_INPUT", kSessionLabel,
              "the refusal lost its code: " + describe(fields));
        check(fields.message == kRefusalMessage, kSessionLabel,
              "the refusal lost its message: " + describe(fields));
        check(fields.kind.has_value() && *fields.kind == kRefusalKind, kSessionLabel,
              "the refusal lost its kind: " + describe(fields));
        check(fields.wire_code.has_value() && *fields.wire_code == kRefusalWireCode, kSessionLabel,
              "the refusal lost its wire code: " + describe(fields));

        // A present empty optional stays present, and an absent one stays absent.
        Result<Buffer> empty_kind = adapter.invoke_tool(kEchoToolId, kEmptyKindArguments);
        const Error& kind = rejection(empty_kind, kSessionLabel,
                                      "a refusal with a present empty kind");
        check(kind.kind.has_value(), kSessionLabel,
              "a present empty `kind` arrived as absent: " + describe(kind));
        check(kind.kind->empty(), kSessionLabel,
              "a present empty `kind` arrived as \"" + *kind.kind + "\"");
        check(!kind.wire_code.has_value(), kSessionLabel,
              "an absent `wire_code` arrived as present: " + describe(kind));

        // A host callback that cannot answer: the enabled configuration throws
        // inside it and the bridge contains the exception, the disabled
        // configuration returns the equivalent `Result` failure. Either way a
        // legal call afterwards still completes.
        Result<Buffer> contained = adapter.invoke_tool(kEchoToolId, kThrowArguments);
        const Error& failure = rejection(contained, kSessionLabel, "a host callback that throws");
        check(failure.status == SPOKE_CONNECT_FFI_REJECTED, kSessionLabel,
              "a throwing host callback reported " + describe(failure));
#if SPOKE_SMOKE_EXCEPTIONS
        check(failure.code.has_value() && *failure.code == "INTERNAL_ERROR", kSessionLabel,
              "the contained exception lost the INTERNAL_ERROR code: " + describe(failure));
        check(failure.kind.has_value() && *failure.kind == "callback", kSessionLabel,
              "the contained exception lost the callback kind: " + describe(failure));
        check(!failure.wire_code.has_value(), kSessionLabel,
              "the contained exception carried a wire code: " + describe(failure));
#else
        check(failure.code.has_value() && *failure.code == "INVALID_INPUT" &&
                  failure.message == kThrowMessage,
              kSessionLabel, "the Result refusal path reported " + describe(failure));
#endif
        const std::string recovered =
            unwrap(adapter.invoke_tool(kEchoToolId, kEchoArguments), kSessionLabel,
                   "RemoteAdapter::invoke_tool after a refusal")
                .str();
        check(recovered.find("\"message\":\"hello\"") != std::string::npos, kSessionLabel,
              "the session stopped serving after a host refusal: " + recovered);

        // Both transports really ran, in both directions.
        check(dialer_counters.sends.load() > 0 && dialer_counters.recvs.load() > 0 &&
                  responder_counters.sends.load() > 0 && responder_counters.recvs.load() > 0,
              kSessionLabel, "the host transport callbacks did not all run");

        // Contract order: close the sessions before releasing the host resources.
        unwrap_ok(adapter.close(), kSessionLabel, "RemoteAdapter::close");
        unwrap_ok(responder.close(), kSessionLabel, "ConnectResponder::close");
        to_responder->close();
        to_dialer->close();
    }

    // Each callback record was released exactly once, and only after both
    // sessions ended and every handle that referenced it was gone.
    check(record_released(dialer_counters, dialer_released), kSessionLabel,
          "the dialer transport record was not released exactly once (releases " +
              std::to_string(dialer_counters.releases.load()) + ")");
    check(record_released(responder_counters, responder_released), kSessionLabel,
          "the responder transport record was not released exactly once (releases " +
              std::to_string(responder_counters.releases.load()) + ")");
    check(record_released(ports_counters, ports_released), kSessionLabel,
          "the ports record was not released exactly once (releases " +
              std::to_string(ports_counters.releases.load()) + ")");
    check(record_released(served_tool_counters, served_tool_released), kSessionLabel,
          "the served tool record was not released exactly once (releases " +
              std::to_string(served_tool_counters.releases.load()) + ")");
    check(record_released(reverse_tool_counters, reverse_tool_released), kSessionLabel,
          "the dialer's tool record was not released exactly once (releases " +
              std::to_string(reverse_tool_counters.releases.load()) + ")");
    // The served records saw exactly the calls the scenario made: the baseline
    // lookup, the two `putKnowledgeEntry` calls and the declining call, and one
    // tool call per invoke — the round trip, the two refusals, the contained
    // failure and the recovery.
    check(ports_counters.calls.load() == 4 && served_tool_counters.calls.load() == 5,
          kSessionLabel,
          "the served records saw " + std::to_string(ports_counters.calls.load()) +
              " ports calls and " + std::to_string(served_tool_counters.calls.load()) +
              " tool calls");
}

}  // namespace

/** Runs the Task 1a convenience-layer group against the shared golden vector. */
void spoke_smoke::run_convenience_values(const Golden& golden) {
    assert_identity_and_versions(golden);
    assert_tampered_hello(golden);
    assert_gates(golden);
    assert_core_objects(golden);
    assert_ownership_and_moves(golden);
    assert_loopback();
    banner(kValuesLabel);
}

/**
 * Runs the Task 1b group: the callback bridges, the adapter and responder
 * wrappers, and the ownership rules they owe the host. The containment banner
 * exists only in the exception-enabled configuration, which is the only one
 * that injects a throwing host callback.
 */
void spoke_smoke::run_convenience_session(const Golden& golden) {
    assert_session(golden);
    banner(kSessionLabel);
#if SPOKE_SMOKE_EXCEPTIONS
    banner(kContainmentLabel);
#endif
}
