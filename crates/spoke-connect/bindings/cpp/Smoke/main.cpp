/*
 * C++17 smoke for the spoke-connect C ABI carrier: the raw-C boundary proof.
 *
 * One of the smoke program's two translation units — the other is
 * `Smoke/convenience.cpp`, which consumes the C++17 convenience layer. Both
 * include `Smoke/support.hpp` (and through it `spoke_connect.hpp`), so the
 * linked program also covers single-header ODR. This unit keeps the raw-C
 * assertions and links the staged native for the host RID
 * (`libspoke_connect_capi.dylib` on macOS, `spoke_connect_capi.dll` plus its
 * import library on Windows), so the same source is the executable boundary
 * proof on both evidence platforms. The runner is
 * `tooling/connect/cpp-smoke.mjs`.
 *
 * Usage: cpp-smoke <path>/crates/spoke-connect/tests/fixtures/golden-hello.json
 *
 * The smoke reads the shared golden vector rather than embedding a copy, hands
 * the parsed value to the other unit, and prints one PASS banner per assertion
 * group — peer id, hello signature, protocol version, loopback ports,
 * rejection/ownership, the convenience layer — followed by the final banner. A
 * failed check prints FAIL and exits non-zero.
 *
 * The callback transport is a host-owned synchronized message queue (see
 * `MessageQueue`): two cross-wired queues carry envelopes between the dialer and
 * the responder. Every callback touches host state only — none of them calls
 * back into the exported loopback helpers.
 */

#include "support.hpp"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <filesystem>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace {

using spoke_smoke::banner;
using spoke_smoke::check;
using spoke_smoke::fail;
using spoke_smoke::Golden;
using spoke_smoke::json_string_field;
using spoke_smoke::load_golden;
using spoke_smoke::read_file;

// ── Assertions ───────────────────────────────────────────────────────────

const char* const kPeerIdLabel = "golden peer-id";
const char* const kHelloLabel = "golden hello signature";
const char* const kProtocolLabel = "protocol version 1";
const char* const kLoopbackLabel = "loopback ports";
const char* const kRejectionLabel = "rejection/ownership";
const char* const kSmokeLabel = "C++ smoke";

/** Fails unless the ABI call succeeded, quoting the error record it filled. */
void require_ok(int32_t status, SpokeConnectError* error, const char* label,
                const std::string& detail);

/** Fails unless the ABI call returned `expected`, quoting the record otherwise. */
void require_status(int32_t status, int32_t expected, SpokeConnectError* error,
                    const char* label, const std::string& detail);

// ── Boundary values ──────────────────────────────────────────────────────

std::string buffer_text(const SpokeConnectBuffer& buffer) {
    if (buffer.data == nullptr || buffer.len == 0) return std::string();
    return std::string(reinterpret_cast<const char*>(buffer.data), buffer.len);
}

/** Copies an owned buffer's text out and releases it in the carrier. */
std::string take_text(SpokeConnectBuffer* buffer) {
    const std::string text = buffer_text(*buffer);
    spoke_connect_buffer_free(buffer);
    return text;
}

/** Renders an error record for a diagnostic message and releases it. */
std::string take_error_text(SpokeConnectError* error) {
    std::string text = "message=\"" + buffer_text(error->message) + "\"";
    if (error->code.data != nullptr) {
        text += " code=\"" + buffer_text(error->code) + "\"";
    }
    if (error->kind.data != nullptr) {
        text += " kind=\"" + buffer_text(error->kind) + "\"";
    }
    if (error->wire_code.data != nullptr) {
        text += " wire_code=\"" + buffer_text(error->wire_code) + "\"";
    }
    spoke_connect_error_free(error);
    return text;
}

void require_ok(int32_t status, SpokeConnectError* error, const char* label,
                const std::string& detail) {
    if (status == SPOKE_CONNECT_OK) return;
    fail(label, detail + " failed with status " + std::to_string(status) + " (" +
                    take_error_text(error) + ")");
}

void require_status(int32_t status, int32_t expected, SpokeConnectError* error,
                    const char* label, const std::string& detail) {
    if (status == expected) {
        spoke_connect_error_free(error);
        return;
    }
    fail(label, detail + ": expected status " + std::to_string(expected) +
                    ", observed " + std::to_string(status) + " (" +
                    take_error_text(error) + ")");
}

std::string slice_text(SpokeConnectSlice slice) {
    if (slice.data == nullptr || slice.len == 0) return std::string();
    return std::string(reinterpret_cast<const char*>(slice.data), slice.len);
}

SpokeConnectSlice slice_of(const std::vector<uint8_t>& bytes) {
    SpokeConnectSlice slice{};
    slice.data = bytes.empty() ? nullptr : bytes.data();
    slice.len = bytes.size();
    return slice;
}

SpokeConnectSlice slice_of(const std::string& text) {
    SpokeConnectSlice slice{};
    slice.data = text.empty() ? nullptr : reinterpret_cast<const uint8_t*>(text.data());
    slice.len = text.size();
    return slice;
}

SpokeConnectOptionalU64 present_u64(uint64_t value) {
    SpokeConnectOptionalU64 optional{};
    optional.present = 1;
    optional.value = value;
    return optional;
}

// ── Golden vector ────────────────────────────────────────────────────────

// The shared `golden-hello.json` struct, its read/parse and the assertion
// primitives live in `support.hpp`, so both smoke translation units share one
// parser and one golden value.

/** Signs the golden hello; Ed25519 signatures are deterministic, so the same
 *  bytes come back for the signature assertion and the tamper rejection. */
std::string sign_golden_hello(const Golden& golden) {
    SpokeConnectBuffer hello{};
    SpokeConnectError error{};
    const SpokeConnectSlice seed = slice_of(golden.seed);
    const SpokeConnectSlice nonce = slice_of(golden.nonce);
    const SpokeConnectSlice manifest = slice_of(golden.manifest_json);
    const int32_t status =
        spoke_connect_sign_hello_ed25519(seed, nonce, manifest, &hello, &error);
    require_ok(status, &error, kHelloLabel, "spoke_connect_sign_hello_ed25519");
    return take_text(&hello);
}

// ── Host-owned synchronized message queue ────────────────────────────────

/** One direction of the queue pair: a bounded-free FIFO with a close flag. */
class MessageQueue {
  public:
    /** Enqueues one envelope and reports whether it was accepted: a push into a
     *  closed queue returns false, so a send that can no longer be delivered is
     *  reported to the carrier instead of being dropped silently. */
    bool push(const std::vector<uint8_t>& envelope) {
        std::lock_guard<std::mutex> guard(mutex_);
        if (closed_) return false;
        envelopes_.push_back(envelope);
        signal_.notify_all();
        return true;
    }

    /** Blocks the calling thread until an envelope arrives or the queue closes. */
    bool pop_blocking(std::vector<uint8_t>* out) {
        std::unique_lock<std::mutex> lock(mutex_);
        for (;;) {
            if (!envelopes_.empty()) {
                *out = envelopes_.front();
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
    std::deque<std::vector<uint8_t>> envelopes_;
    bool closed_ = false;
};

/** One end of a queue pair: outbound pushes, inbound blocks. */
struct Endpoint {
    MessageQueue* outbound;
    MessageQueue* inbound;

    bool send(const std::vector<uint8_t>& envelope) { return outbound->push(envelope); }
    bool recv(std::vector<uint8_t>* out) { return inbound->pop_blocking(out); }
    void close() {
        outbound->close();
        inbound->close();
    }
};

/** What the host observed about the callbacks the carrier ran; host-owned and
 *  read by the assertions after the carrier context itself is destroyed. */
struct CallbackCounters {
    std::atomic<long> send{0};
    std::atomic<long> recv{0};
    std::atomic<long> recv_closed{0};
    std::atomic<long> release{0};
    std::atomic<long> close{0};
    std::atomic<long> destroy{0};
};

/** The host transport context behind `SpokeConnectTransportTable`; the carrier
 *  owns it after a successful `spoke_connect_transport_new` and destroys it
 *  exactly once. */
struct HostTransport {
    Endpoint endpoint;
    CallbackCounters* counters;
};

/** Releases a buffer this host allocated for the carrier; the carrier calls it
 *  exactly once per populated buffer. */
void SPOKE_CONNECT_CALL release_owned_buffer(void* release_context, const uint8_t* data,
                                             size_t len) {
    CallbackCounters* counters = static_cast<CallbackCounters*>(release_context);
    if (counters != nullptr) counters->release.fetch_add(1);
    if (data != nullptr && len != 0) delete[] data;
}

/** Releases host static storage: the pointer stays owned by the host. */
void SPOKE_CONNECT_CALL release_static(void* /*release_context*/, const uint8_t* /*data*/,
                                       size_t /*len*/) {}

void write_static_error(SpokeConnectForeignError* out_error, const char* message) {
    if (out_error == nullptr) return;
    SpokeConnectForeignError record{};
    record.message.data = reinterpret_cast<const uint8_t*>(message);
    record.message.len = std::strlen(message);
    record.message.release = &release_static;
    *out_error = record;
}

void write_foreign_owned(SpokeConnectForeignBuffer* out_buffer, const std::string& payload,
                         CallbackCounters* counters) {
    if (out_buffer == nullptr || payload.empty()) return;
    uint8_t* data = new uint8_t[payload.size()];
    std::memcpy(data, payload.data(), payload.size());
    SpokeConnectForeignBuffer buffer{};
    buffer.data = data;
    buffer.len = payload.size();
    buffer.release_context = counters;
    buffer.release = &release_owned_buffer;
    *out_buffer = buffer;
}

extern "C" {

int32_t SPOKE_CONNECT_CALL transport_send(void* user_data, SpokeConnectSlice envelope,
                                          SpokeConnectForeignError* out_error) {
    HostTransport* host = static_cast<HostTransport*>(user_data);
    host->counters->send.fetch_add(1);
    std::vector<uint8_t> bytes;
    if (envelope.data != nullptr && envelope.len != 0) {
        bytes.assign(envelope.data, envelope.data + envelope.len);
    }
    // A closed queue cannot deliver, so the contract's transport-closed status
    // (with an error record) replaces the success that would acknowledge a
    // dropped envelope.
    if (!host->endpoint.send(bytes)) {
        write_static_error(out_error, "host transport closed");
        return SPOKE_CONNECT_TRANSPORT_CLOSED;
    }
    return SPOKE_CONNECT_OK;
}

int32_t SPOKE_CONNECT_CALL transport_recv(void* user_data,
                                          SpokeConnectForeignBuffer* out_envelope,
                                          SpokeConnectForeignError* out_error) {
    HostTransport* host = static_cast<HostTransport*>(user_data);
    host->counters->recv.fetch_add(1);
    std::vector<uint8_t> envelope;
    if (!host->endpoint.recv(&envelope)) {
        host->counters->recv_closed.fetch_add(1);
        write_static_error(out_error, "host transport closed");
        return SPOKE_CONNECT_TRANSPORT_CLOSED;
    }
    if (out_envelope == nullptr) return SPOKE_CONNECT_OK;
    if (envelope.empty()) {
        SpokeConnectForeignBuffer empty{};
        *out_envelope = empty;
        return SPOKE_CONNECT_OK;
    }
    uint8_t* data = new uint8_t[envelope.size()];
    std::memcpy(data, envelope.data(), envelope.size());
    SpokeConnectForeignBuffer buffer{};
    buffer.data = data;
    buffer.len = envelope.size();
    buffer.release_context = host->counters;
    buffer.release = &release_owned_buffer;
    *out_envelope = buffer;
    return SPOKE_CONNECT_OK;
}

int32_t SPOKE_CONNECT_CALL transport_close(void* user_data,
                                           SpokeConnectForeignError* /*out_error*/) {
    HostTransport* host = static_cast<HostTransport*>(user_data);
    host->counters->close.fetch_add(1);
    host->endpoint.close();
    return SPOKE_CONNECT_OK;
}

void SPOKE_CONNECT_CALL transport_destroy(void* user_data) {
    HostTransport* host = static_cast<HostTransport*>(user_data);
    host->counters->destroy.fetch_add(1);
    delete host;
}

}  // extern "C"

SpokeConnectTransportTable transport_table() {
    SpokeConnectTransportTable table{};
    table.send = &transport_send;
    table.recv = &transport_recv;
    table.close = &transport_close;
    table.destroy = &transport_destroy;
    return table;
}

// ── Serving side: the ports callback the responder runs ──────────────────

const char* const kEntryIdText = "cpp-smoke-entry-0001";
const char* const kEntryCanonicalName = "C++ smoke knowledge entry";
const char* const kServedEntryJson =
    "{\"schema_version\":1,\"entry_id\":\"cpp-smoke-entry-0001\",\"entry_type\":\"note\","
    "\"canonical_name\":\"C++ smoke knowledge entry\",\"status\":\"confirmed\","
    "\"body\":{\"summary\":\"served over the host-owned loopback\"},\"extensions\":{}}";
const char* const kDeclineCode = "op_unsupported";
const char* const kDeclineMessage = "the C++ smoke serves the baseline knowledge lookup";

const std::string kEntryId(kEntryIdText);

/** The state the served ports callback shares with the assertions. */
struct HostPortsState {
    std::mutex mutex;
    long calls = 0;
    std::string last_entry_id;
};

/** The host ports context behind `SpokeConnectPortsHandlerTable`. */
struct HostPorts {
    HostPortsState* state;
    CallbackCounters* counters;
};

/** The observed ports-callback state, read after the round trip returned. */
struct PortsSnapshot {
    long calls;
    std::string last_entry_id;
};

PortsSnapshot snapshot(HostPortsState& state) {
    std::lock_guard<std::mutex> guard(state.mutex);
    PortsSnapshot observed{};
    observed.calls = state.calls;
    observed.last_entry_id = state.last_entry_id;
    return observed;
}

void write_decline(SpokeConnectForeignError* out_error) {
    if (out_error == nullptr) return;
    SpokeConnectForeignError record{};
    record.code.data = reinterpret_cast<const uint8_t*>(kDeclineCode);
    record.code.len = std::strlen(kDeclineCode);
    record.code.release = &release_static;
    record.message.data = reinterpret_cast<const uint8_t*>(kDeclineMessage);
    record.message.len = std::strlen(kDeclineMessage);
    record.message.release = &release_static;
    *out_error = record;
}

extern "C" {

/** Serves `getKnowledgeEntry`: records the entry id and answers the canned
 *  entry. Runs on the carrier's blocking pool. */
int32_t SPOKE_CONNECT_CALL ports_get_knowledge_entry(void* user_data,
                                                     SpokeConnectSlice input_json,
                                                     SpokeConnectForeignBuffer* out_json,
                                                     SpokeConnectForeignError* /*out_error*/) {
    HostPorts* host = static_cast<HostPorts*>(user_data);
    const std::string entry_id = slice_text(input_json);
    {
        std::lock_guard<std::mutex> guard(host->state->mutex);
        host->state->calls += 1;
        host->state->last_entry_id = entry_id;
    }
    write_foreign_owned(out_json, kServedEntryJson, host->counters);
    return SPOKE_CONNECT_OK;
}

/** SpokeConnectPortsTextFn decline for an op this host does not serve. */
int32_t SPOKE_CONNECT_CALL ports_decline_text(void* /*user_data*/,
                                              SpokeConnectSlice /*input_json*/,
                                              SpokeConnectForeignBuffer* /*out_json*/,
                                              SpokeConnectForeignError* out_error) {
    write_decline(out_error);
    return SPOKE_CONNECT_FFI_REJECTED;
}

/** SpokeConnectPortsRevisionFn decline. */
int32_t SPOKE_CONNECT_CALL ports_decline_revision(void* /*user_data*/,
                                                  SpokeConnectSlice /*input_json*/,
                                                  SpokeConnectOptionalU64 /*expected_base_revision*/,
                                                  SpokeConnectForeignBuffer* /*out_json*/,
                                                  SpokeConnectForeignError* out_error) {
    write_decline(out_error);
    return SPOKE_CONNECT_FFI_REJECTED;
}

/** SpokeConnectPortsListRulesFn decline. */
int32_t SPOKE_CONNECT_CALL ports_decline_list_rules(void* /*user_data*/,
                                                    const SpokeConnectSlice* /*rule_refs*/,
                                                    size_t /*rule_refs_count*/,
                                                    SpokeConnectForeignBuffer* /*out_json*/,
                                                    SpokeConnectForeignError* out_error) {
    write_decline(out_error);
    return SPOKE_CONNECT_FFI_REJECTED;
}

/** SpokeConnectPortsNoInputFn decline. */
int32_t SPOKE_CONNECT_CALL ports_decline_no_input(void* /*user_data*/,
                                                  SpokeConnectForeignBuffer* /*out_json*/,
                                                  SpokeConnectForeignError* out_error) {
    write_decline(out_error);
    return SPOKE_CONNECT_FFI_REJECTED;
}

void SPOKE_CONNECT_CALL ports_destroy(void* user_data) {
    HostPorts* host = static_cast<HostPorts*>(user_data);
    host->counters->destroy.fetch_add(1);
    delete host;
}

}  // extern "C"

SpokeConnectPortsHandlerTable ports_table() {
    SpokeConnectPortsHandlerTable table{};
    table.get_knowledge_entry = &ports_get_knowledge_entry;
    table.put_knowledge_entry = &ports_decline_revision;
    table.get_relation = &ports_decline_text;
    table.put_relation = &ports_decline_revision;
    table.list_knowledge_entries = &ports_decline_text;
    table.list_timeline_events = &ports_decline_text;
    table.put_findings = &ports_decline_text;
    table.list_rules = &ports_decline_list_rules;
    table.list_peer_host_capability_manifests = &ports_decline_no_input;
    table.project = &ports_decline_text;
    table.compute = &ports_decline_text;
    table.list_fork_timeline_events = &ports_decline_text;
    table.extract = &ports_decline_text;
    table.destroy = &ports_destroy;
    return table;
}

// ── Small helpers ────────────────────────────────────────────────────────

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

std::string adapter_state(const SpokeConnectRemoteAdapter* adapter) {
    SpokeConnectBuffer state{};
    SpokeConnectError error{};
    const int32_t status = spoke_connect_remote_adapter_state(adapter, &state, &error);
    require_ok(status, &error, kLoopbackLabel, "spoke_connect_remote_adapter_state");
    return take_text(&state);
}

std::string responder_state(const SpokeConnectResponder* responder) {
    SpokeConnectBuffer state{};
    SpokeConnectError error{};
    const int32_t status = spoke_connect_responder_state(responder, &state, &error);
    require_ok(status, &error, kLoopbackLabel, "spoke_connect_responder_state");
    return take_text(&state);
}

// ── Assertion groups ─────────────────────────────────────────────────────

/** The wire peer id derived from the golden public key matches the vector. */
void assert_golden_peer_id(const Golden& golden) {
    SpokeConnectBuffer peer_id{};
    SpokeConnectError error{};
    const SpokeConnectSlice pubkey = slice_of(golden.pubkey);
    const int32_t status =
        spoke_connect_derive_peer_id_from_ed25519_pubkey(pubkey, &peer_id, &error);
    require_ok(status, &error, kPeerIdLabel,
               "spoke_connect_derive_peer_id_from_ed25519_pubkey");
    const std::string derived = take_text(&peer_id);
    check(derived == golden.peer_id, kPeerIdLabel,
          "the derived peer id \"" + derived + "\" differs from the golden vector \"" +
              golden.peer_id + "\"");
    banner(kPeerIdLabel);
}

/** The hello signed from the golden inputs carries the pinned signature, and the
 *  carrier verifies it against the golden public key. */
void assert_golden_hello_signature(const Golden& golden) {
    const std::string hello_json = sign_golden_hello(golden);
    const std::string signature = json_string_field(hello_json, "signature");
    check(signature == golden.signature_b64u, kHelloLabel,
          "the signed hello carries signature \"" + signature +
              "\" instead of the pinned golden signature \"" + golden.signature_b64u + "\"");
    const std::string hello_peer_id = json_string_field(hello_json, "peer_id");
    check(hello_peer_id == golden.peer_id, kHelloLabel,
          "the signed hello carries peer id \"" + hello_peer_id + "\"");

    SpokeConnectError error{};
    const SpokeConnectSlice pubkey = slice_of(golden.pubkey);
    const SpokeConnectSlice peer_id = slice_of(golden.peer_id);
    const SpokeConnectSlice hello = slice_of(hello_json);
    const int32_t status =
        spoke_connect_verify_hello_ed25519(pubkey, peer_id, hello, &error);
    require_ok(status, &error, kHelloLabel, "spoke_connect_verify_hello_ed25519");
    banner(kHelloLabel);
}

/** The boundary revision and the connect hello protocol version are both 1. */
void assert_protocol_version() {
    SpokeConnectError error{};
    uint64_t protocol_version = 0;
    int32_t status = spoke_connect_protocol_version(&protocol_version, &error);
    require_ok(status, &error, kProtocolLabel, "spoke_connect_protocol_version");
    check(protocol_version == 1, kProtocolLabel,
          "spoke_connect_protocol_version reported " + std::to_string(protocol_version));

    uint64_t abi_version = 0;
    status = spoke_connect_abi_version(&abi_version, &error);
    require_ok(status, &error, kProtocolLabel, "spoke_connect_abi_version");
    check(abi_version == 1, kProtocolLabel,
          "spoke_connect_abi_version reported " + std::to_string(abi_version));
    banner(kProtocolLabel);
}

/** One envelope round trip in both directions over the exported loopback pair. */
void assert_loopback_pair_round_trip() {
    SpokeConnectError error{};
    SpokeConnectLoopbackTransportPair* pair = nullptr;
    require_ok(spoke_connect_loopback_transport_pair_new(&pair, &error), &error,
               kLoopbackLabel, "spoke_connect_loopback_transport_pair_new");

    SpokeConnectLoopbackTransport* client = nullptr;
    SpokeConnectLoopbackTransport* server = nullptr;
    require_ok(spoke_connect_loopback_transport_pair_client(pair, &client, &error), &error,
               kLoopbackLabel, "spoke_connect_loopback_transport_pair_client");
    require_ok(spoke_connect_loopback_transport_pair_server(pair, &server, &error), &error,
               kLoopbackLabel, "spoke_connect_loopback_transport_pair_server");

    const std::string to_server = "cpp-smoke-loopback-envelope-0001";
    const std::string to_client = "cpp-smoke-loopback-envelope-0002";

    require_ok(spoke_connect_loopback_transport_send(client, slice_of(to_server), &error),
               &error, kLoopbackLabel, "spoke_connect_loopback_transport_send (client)");
    SpokeConnectBuffer received_server{};
    require_ok(spoke_connect_loopback_transport_recv(server, &received_server, &error),
               &error, kLoopbackLabel, "spoke_connect_loopback_transport_recv (server)");
    check(take_text(&received_server) == to_server, kLoopbackLabel,
          "the server end received different bytes than the client end sent");

    require_ok(spoke_connect_loopback_transport_send(server, slice_of(to_client), &error),
               &error, kLoopbackLabel, "spoke_connect_loopback_transport_send (server)");
    SpokeConnectBuffer received_client{};
    require_ok(spoke_connect_loopback_transport_recv(client, &received_client, &error),
               &error, kLoopbackLabel, "spoke_connect_loopback_transport_recv (client)");
    check(take_text(&received_client) == to_client, kLoopbackLabel,
          "the client end received different bytes than the server end sent");

    require_ok(spoke_connect_loopback_transport_close(client, &error), &error,
               kLoopbackLabel, "spoke_connect_loopback_transport_close");
    spoke_connect_loopback_transport_free(client);
    spoke_connect_loopback_transport_free(server);
    spoke_connect_loopback_transport_pair_free(pair);
}

/**
 * A dialer and a responder over two cross-wired host queues, then one baseline
 * ports call across that loopback. Both ends use the golden identity, so the
 * fixture that pins the golden assertions also supplies the session identities.
 *
 * The scenario is a fixed handshake plus one invoke/response pair, so it carries
 * a deterministic number of envelopes in each direction: the counts below are
 * what every host-populated callback buffer must be released back exactly once
 * for (A2 §C), and `ports_counters.release` is the served ports result buffer.
 */
constexpr long kDialerEnvelopes = 2;
constexpr long kResponderEnvelopes = 3;

void assert_host_queue_loopback_ports(const Golden& golden) {
    MessageQueue dialer_to_responder;
    MessageQueue responder_to_dialer;
    const Endpoint dialer_endpoint{&dialer_to_responder, &responder_to_dialer};
    const Endpoint responder_endpoint{&responder_to_dialer, &dialer_to_responder};

    CallbackCounters dialer_counters;
    CallbackCounters responder_counters;
    CallbackCounters ports_counters;
    HostPortsState ports_state;

    SpokeConnectError error{};

    const SpokeConnectTransportTable transport = transport_table();
    const SpokeConnectPortsHandlerTable ports = ports_table();

    HostTransport* responder_host = new HostTransport{responder_endpoint, &responder_counters};
    SpokeConnectTransport* responder_transport = nullptr;
    require_ok(
        spoke_connect_transport_new(&transport, responder_host, &responder_transport, &error),
        &error, kLoopbackLabel, "spoke_connect_transport_new (responder)");

    HostPorts* ports_host = new HostPorts{&ports_state, &ports_counters};
    SpokeConnectPortsHandler* ports_handler = nullptr;
    require_ok(spoke_connect_ports_handler_new(&ports, ports_host, &ports_handler, &error),
               &error, kLoopbackLabel, "spoke_connect_ports_handler_new");

    const SpokeConnectSlice seed = slice_of(golden.seed);
    const SpokeConnectSlice manifest = slice_of(golden.manifest_json);
    const SpokeConnectSlice pubkey = slice_of(golden.pubkey);
    const SpokeConnectSlice peer_id = slice_of(golden.peer_id);
    const SpokeConnectSlice allowlist[] = {peer_id};
    const SpokeConnectPeerKey peer_keys[] = {SpokeConnectPeerKey{peer_id, pubkey}};
    const SpokeConnectOptionalU64 invoke_timeout = present_u64(5000);

    SpokeConnectResponder* responder = nullptr;
    require_ok(
        spoke_connect_responder_new(responder_transport, seed, manifest, allowlist, 1, peer_keys,
                                    1, ports_handler, invoke_timeout, &responder, &error),
        &error, kLoopbackLabel, "spoke_connect_responder_new");

    HostTransport* dialer_host = new HostTransport{dialer_endpoint, &dialer_counters};
    SpokeConnectTransport* dialer_transport = nullptr;
    require_ok(spoke_connect_transport_new(&transport, dialer_host, &dialer_transport, &error),
               &error, kLoopbackLabel, "spoke_connect_transport_new (dialer)");

    SpokeConnectRemoteAdapter* adapter = nullptr;
    require_ok(spoke_connect_remote_adapter_new(dialer_transport, seed, manifest, pubkey,
                                                allowlist, 1, invoke_timeout, &adapter, &error),
               &error, kLoopbackLabel, "spoke_connect_remote_adapter_new");

    const std::string dialer_state = adapter_state(adapter);
    const bool responder_established =
        wait_for([&responder] { return responder_state(responder) == "Established"; },
                 std::chrono::milliseconds(5000));
    if (dialer_state != "Established" || !responder_established) {
        fail(kLoopbackLabel, "the loopback session did not reach Established: dialer \"" +
                                 dialer_state + "\", responder \"" + responder_state(responder) +
                                 "\"");
    }

    SpokeConnectBuffer entry{};
    require_ok(spoke_connect_remote_adapter_get_knowledge_entry(adapter, slice_of(kEntryId),
                                                                &entry, &error),
               &error, kLoopbackLabel,
               "spoke_connect_remote_adapter_get_knowledge_entry");
    const std::string entry_json = take_text(&entry);
    check(entry_json.find("\"entry_id\":\"" + kEntryId + "\"") != std::string::npos,
          kLoopbackLabel, "the round-tripped entry carries no entry id: " + entry_json);
    check(entry_json.find("\"canonical_name\":\"" + std::string(kEntryCanonicalName) + "\"") !=
              std::string::npos,
          kLoopbackLabel, "the round-tripped entry carries no canonical name: " + entry_json);

    const PortsSnapshot observed = snapshot(ports_state);
    check(observed.calls == 1, kLoopbackLabel,
          "the served ports callback ran " + std::to_string(observed.calls) + " times");
    check(observed.last_entry_id == kEntryId, kLoopbackLabel,
          "the served ports callback saw entry id \"" + observed.last_entry_id + "\"");
    check(dialer_counters.send.load() > 0 && dialer_counters.recv.load() > 0, kLoopbackLabel,
          "the dialer transport callbacks did not both run");
    check(responder_counters.send.load() > 0 && responder_counters.recv.load() > 0,
          kLoopbackLabel, "the responder transport callbacks did not both run");

    require_ok(spoke_connect_remote_adapter_close(adapter, &error), &error, kLoopbackLabel,
               "spoke_connect_remote_adapter_close");
    spoke_connect_remote_adapter_free(adapter);
    require_ok(spoke_connect_responder_close(responder, &error), &error, kLoopbackLabel,
               "spoke_connect_responder_close");
    spoke_connect_responder_free(responder);
    spoke_connect_ports_handler_free(ports_handler);
    spoke_connect_transport_free(dialer_transport);
    spoke_connect_transport_free(responder_transport);

    // The host drops the connection: any receive still in flight returns
    // transport-closed, which lets the carrier release its last reference.
    dialer_to_responder.close();
    responder_to_dialer.close();

    check(wait_for([&dialer_counters] { return dialer_counters.destroy.load() == 1; },
                   std::chrono::milliseconds(5000)),
          kLoopbackLabel, "the dialer transport context was not destroyed exactly once");
    check(wait_for([&responder_counters] { return responder_counters.destroy.load() == 1; },
                   std::chrono::milliseconds(5000)),
          kLoopbackLabel, "the responder transport context was not destroyed exactly once");
    check(wait_for([&ports_counters] { return ports_counters.destroy.load() == 1; },
                   std::chrono::milliseconds(5000)),
          kLoopbackLabel, "the ports context was not destroyed exactly once");

    // Every populated callback buffer transfers to the carrier and comes back
    // released exactly once (A2 §C). No callback can still be in flight here
    // (each context was destroyed exactly once above), so the deterministic
    // scenario pins both transport counters and the served ports result buffer
    // to exact counts: the dialer received every envelope the responder sent
    // and vice versa, and the ports callback's populated result was released
    // once — not merely "some buffer was released".
    const long dialer_releases = dialer_counters.release.load();
    const long responder_releases = responder_counters.release.load();
    const long ports_releases = ports_counters.release.load();
    check(dialer_releases == kResponderEnvelopes, kLoopbackLabel,
          "the dialer released " + std::to_string(dialer_releases) + " of the " +
              std::to_string(kResponderEnvelopes) + " responder envelope buffers");
    check(responder_releases == kDialerEnvelopes, kLoopbackLabel,
          "the responder released " + std::to_string(responder_releases) + " of the " +
              std::to_string(kDialerEnvelopes) + " dialer envelope buffers");
    check(ports_releases == 1, kLoopbackLabel,
          "the served ports result buffer was released " + std::to_string(ports_releases) +
              " times instead of exactly once");
}

/** One loopback session plus one baseline ports round trip over it. */
void assert_loopback_ports(const Golden& golden) {
    assert_loopback_pair_round_trip();
    assert_host_queue_loopback_ports(golden);
    banner(kLoopbackLabel);
}

/** Rejected input leaves ownership with the caller, an accepted handle is
 *  destroyed exactly once, an undeliverable send reports transport-closed, and
 *  a tampered hello fails verification. */
void assert_rejection_and_ownership(const Golden& golden) {
    SpokeConnectError error{};

    MessageQueue discarded_outbound;
    MessageQueue discarded_inbound;
    const Endpoint discarded_endpoint{&discarded_outbound, &discarded_inbound};

    // A table missing a callback is rejected without taking the context.
    SpokeConnectTransportTable incomplete = transport_table();
    incomplete.destroy = nullptr;
    CallbackCounters rejected_counters;
    HostTransport* rejected_host = new HostTransport{discarded_endpoint, &rejected_counters};
    SpokeConnectTransport* rejected_transport = nullptr;
    require_status(spoke_connect_transport_new(&incomplete, rejected_host, &rejected_transport,
                                               &error),
                   SPOKE_CONNECT_INVALID_ARGUMENT, &error, kRejectionLabel,
                   "an incomplete transport table");
    check(rejected_transport == nullptr, kRejectionLabel,
          "a rejected transport table produced a handle");
    check(rejected_counters.destroy.load() == 0, kRejectionLabel,
          "destroy ran for a rejected transport table (ownership stays with the caller)");
    delete rejected_host;

    // An accepted table hands the context over; release destroys it exactly once.
    const SpokeConnectTransportTable complete = transport_table();
    CallbackCounters accepted_counters;
    HostTransport* accepted_host = new HostTransport{discarded_endpoint, &accepted_counters};
    SpokeConnectTransport* accepted_transport = nullptr;
    require_ok(spoke_connect_transport_new(&complete, accepted_host, &accepted_transport, &error),
               &error, kRejectionLabel, "spoke_connect_transport_new");
    check(accepted_counters.destroy.load() == 0, kRejectionLabel,
          "the carrier destroyed an accepted context before the handle was released");
    spoke_connect_transport_free(accepted_transport);
    check(wait_for([&accepted_counters] { return accepted_counters.destroy.load() == 1; },
                   std::chrono::milliseconds(5000)),
          kRejectionLabel, "an accepted transport context was not destroyed exactly once");
    spoke_connect_transport_free(nullptr);

    // A send that can no longer be delivered reports the contract's
    // transport-closed status with an error record, so a concurrent close/send
    // cannot lose an envelope while acknowledging success. The record points at
    // static storage with the no-op release, so it needs no free.
    MessageQueue closed_outbound;
    MessageQueue closed_inbound;
    Endpoint closed_endpoint{&closed_outbound, &closed_inbound};
    closed_endpoint.close();
    CallbackCounters closed_counters;
    HostTransport closed_host{closed_endpoint, &closed_counters};
    SpokeConnectForeignError send_error{};
    const std::string undeliverable = "cpp-smoke-undeliverable-envelope";
    const int32_t send_status =
        transport_send(&closed_host, slice_of(undeliverable), &send_error);
    check(send_status == SPOKE_CONNECT_TRANSPORT_CLOSED, kRejectionLabel,
          "a send on a closed host transport returned " + std::to_string(send_status) +
              " instead of transport-closed (" +
              std::to_string(SPOKE_CONNECT_TRANSPORT_CLOSED) + ")");
    check(send_error.message.data != nullptr && send_error.message.len != 0, kRejectionLabel,
          "a send on a closed host transport carried no error record");

    // A tampered hello no longer verifies against the golden public key.
    std::string tampered = sign_golden_hello(golden);
    const size_t role_at = tampered.find("\"data-store\"");
    check(role_at != std::string::npos, kRejectionLabel,
          "the signed golden hello carries no role field to tamper with");
    tampered.replace(role_at, std::strlen("\"data-store\""), "\"checker\"");
    const SpokeConnectSlice pubkey = slice_of(golden.pubkey);
    const SpokeConnectSlice peer_id = slice_of(golden.peer_id);
    const SpokeConnectSlice hello = slice_of(tampered);
    require_status(spoke_connect_verify_hello_ed25519(pubkey, peer_id, hello, &error),
                   SPOKE_CONNECT_INVALID_HELLO_SIGNATURE, &error, kRejectionLabel,
                   "a tampered hello");
    banner(kRejectionLabel);
}

std::string resolve_fixture_path(const char* raw_path) {
    namespace fs = std::filesystem;
    std::error_code ec;

    const fs::path base = fs::weakly_canonical("crates/spoke-connect/tests/fixtures", ec);
    if (ec) {
        fail(kFixtureLabel, "cannot resolve fixtures base directory");
    }

    const fs::path candidate = fs::weakly_canonical(fs::path(raw_path), ec);
    if (ec) {
        fail(kFixtureLabel, std::string("cannot resolve fixture path: ") + raw_path);
    }

    const std::string base_s = base.generic_string();
    const std::string candidate_s = candidate.generic_string();
    const bool in_base =
        candidate_s.size() >= base_s.size() &&
        candidate_s.compare(0, base_s.size(), base_s) == 0 &&
        (candidate_s.size() == base_s.size() || candidate_s[base_s.size()] == '/');
    if (!in_base) {
        fail(kFixtureLabel, std::string("fixture path escapes fixtures directory: ") + raw_path);
    }

    return candidate.string();
}

}  // namespace

int main(int argc, char** argv) {
    if (argc != 2) {
        std::fprintf(stderr,
                     "usage: %s "
                     "<path>/crates/spoke-connect/tests/fixtures/golden-hello.json\n",
                     argv[0]);
        return 2;
    }
    const std::string fixture_path = resolve_fixture_path(argv[1]);
    const Golden golden = load_golden(read_file(fixture_path));
    assert_golden_peer_id(golden);
    assert_golden_hello_signature(golden);
    assert_protocol_version();
    assert_loopback_ports(golden);
    assert_rejection_and_ownership(golden);
    spoke_smoke::run_convenience_values(golden);
    spoke_smoke::run_convenience_session(golden);
    banner(kSmokeLabel);
    return 0;
}
