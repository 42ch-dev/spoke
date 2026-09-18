/*
 * C++17 convenience-layer smoke: the value and ownership layer, the core free
 * functions, the three core session objects and the loopback pair and ends,
 * all over `spoke_connect.hpp`.
 *
 * The unit is linked with `Smoke/main.cpp` into one smoke program: both
 * translation units include the convenience header (through `support.hpp`), so
 * the build also covers single-header ODR. `main.cpp` reads the shared golden
 * vector once and hands it over — this unit never re-reads or re-transcribes it.
 *
 * The runner is `tooling/connect/cpp-smoke.mjs`; the group banner is printed
 * only after every assertion in the unit has passed, and a failed check prints
 * FAIL and exits non-zero.
 */

#include "support.hpp"

#include <atomic>
#include <chrono>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <thread>
#include <utility>
#include <vector>

namespace {

using spoke::connect::Buffer;
using spoke::connect::Error;
using spoke::connect::Result;
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
T unwrap(Result<T>&& result, const std::string& detail) {
    if (!result.has_value()) fail(kValuesLabel, detail + " failed: " + describe(result.error()));
    return std::move(result).value();
}

/** The `Result<void>` flavour of `unwrap`. */
void unwrap_ok(Result<void>&& result, const std::string& detail) {
    if (!result.has_value()) fail(kValuesLabel, detail + " failed: " + describe(result.error()));
}

/** Fails unless the call reported exactly `expected_status`. */
template <typename T>
void require_status(Result<T>& result, int32_t expected_status, const std::string& detail) {
    if (result.has_value()) fail(kValuesLabel, detail + " unexpectedly succeeded");
    const Error& error = result.error();
    if (error.status != expected_status) fail(kValuesLabel, detail + " reported " + describe(error));
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
