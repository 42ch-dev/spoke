/*
 * spoke-connect C++17 convenience layer over the hand-written C ABI.
 *
 * `spoke_connect.hpp` includes the sibling `spoke_connect.h` and wraps the
 * shipped C surface in `spoke::connect`: move-only ownership for every owned
 * buffer and opaque handle, borrowed text views, explicit copies, structured
 * `Result` values and host callback bridges. Everything is class-body-inline,
 * `inline` or template — the header is the whole library, and no call reaches
 * past an existing declaration in `spoke_connect.h`.
 *
 * Language floor C++17 with no RTTI. The consumer default builds with
 * exceptions disabled (`-fno-exceptions` / `/EHs-c-`); exception-enabled builds
 * use the same error API, and no public declaration here depends on the
 * standard-library configuration macro.
 *
 * Ownership rules: a destructor only releases — it never closes a session. An
 * owned output buffer arrives as a `Buffer` (zero-copy; `view()` borrows,
 * `str()` copies), a handle arrives as its own move-only class, and both are
 * released exactly once through the matching carrier release function.
 */

#ifndef SPOKE_CONNECT_HPP
#define SPOKE_CONNECT_HPP

#include "spoke_connect.h"

#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <variant>

namespace spoke::connect {

namespace detail {

/**
 * Aborts on a caller programming error — a checked access on the wrong
 * branch of a `Result`. Failing fast in every build configuration keeps the
 * result the single error channel instead of letting a second,
 * exception-shaped one appear in exception-enabled builds.
 */
[[noreturn]] inline void contract_violation(const char* message) noexcept {
    std::fputs("spoke::connect contract violation: ", stderr);
    std::fputs(message, stderr);
    std::fputc('\n', stderr);
    std::fflush(stderr);
    std::abort();
}

/** Copies owned buffer text; a zero buffer reads as an empty string. */
[[nodiscard]] inline std::string text_of(const SpokeConnectBuffer& buffer) {
    if (buffer.data == nullptr || buffer.len == 0) return std::string();
    return std::string(reinterpret_cast<const char*>(buffer.data), buffer.len);
}

/**
 * Copies owned buffer text where the pointer's presence is the field's
 * presence: a NULL pointer is absent, a non-NULL pointer with `len == 0` is a
 * present empty string.
 */
[[nodiscard]] inline std::optional<std::string> optional_text_of(
    const SpokeConnectBuffer& buffer) {
    if (buffer.data == nullptr) return std::nullopt;
    return std::string(reinterpret_cast<const char*>(buffer.data), buffer.len);
}

/**
 * Owns one call's C error out-record. The record is released even when copying
 * its fields into the C++ `Error` fails, so the carrier record cannot leak.
 */
class CErrorRecord {
  public:
    CErrorRecord() noexcept = default;
    CErrorRecord(const CErrorRecord&) = delete;
    CErrorRecord& operator=(const CErrorRecord&) = delete;
    ~CErrorRecord() { spoke_connect_error_free(&record_); }

    [[nodiscard]] SpokeConnectError* out() noexcept { return &record_; }
    [[nodiscard]] const SpokeConnectError& get() const noexcept { return record_; }

  private:
    SpokeConnectError record_{};
};

/**
 * Owns one opaque carrier handle. Move-only; the destructor releases the handle
 * through the carrier's matching release function and never calls `close`, so
 * ending a session stays an explicit call. A moved-from handle is empty.
 */
template <typename Handle, void (SPOKE_CONNECT_CALL *Free)(Handle*)>
class HandleBase {
  public:
    HandleBase(const HandleBase&) = delete;
    HandleBase& operator=(const HandleBase&) = delete;

    HandleBase(HandleBase&& other) noexcept : handle_(other.handle_) {
        other.handle_ = nullptr;
    }

    HandleBase& operator=(HandleBase&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    ~HandleBase() { reset(); }

    /** Borrows the owned handle for raw-C interop; this object keeps ownership. */
    [[nodiscard]] Handle* get() const noexcept { return handle_; }
    [[nodiscard]] explicit operator bool() const noexcept { return handle_ != nullptr; }

    /** Transfers ownership to the caller; this object becomes empty. */
    [[nodiscard]] Handle* release() noexcept {
        Handle* released = handle_;
        handle_ = nullptr;
        return released;
    }

    /**
     * Takes ownership of a raw handle for raw-C interop, releasing the
     * currently owned one first. A NULL handle is accepted and leaves this
     * object empty.
     */
    void adopt(Handle* handle) noexcept {
        reset();
        handle_ = handle;
    }

  protected:
    HandleBase() noexcept = default;
    explicit HandleBase(Handle* handle) noexcept : handle_(handle) {}

  private:
    void reset() noexcept {
        if (handle_ != nullptr) {
            Free(handle_);
            handle_ = nullptr;
        }
    }

    Handle* handle_ = nullptr;
};

}  // namespace detail

/**
 * One failure: the C status the call returned plus the structured fields the
 * carrier recorded. The status values are the existing `SPOKE_CONNECT_*`
 * constants — there is no second status enum. `code` / `kind` / `wire_code`
 * follow the C record's pointer presence, so an optional field the carrier
 * filled with an empty string stays present.
 */
struct Error {
    int32_t status = SPOKE_CONNECT_OK;
    std::string message;
    std::optional<std::string> code;
    std::optional<std::string> kind;
    std::optional<std::string> wire_code;
    uint64_t expected = 0;
    int64_t actual = 0;
};

/**
 * Owns one carrier-allocated output buffer, including its trailing NUL byte.
 * Move-only: exactly one owner releases the record, and a zero buffer is a
 * no-op release. The destructor only frees — there is nothing to close.
 */
class Buffer {
  public:
    Buffer() noexcept = default;
    Buffer(Buffer&& other) noexcept : record_(other.record_) {
        other.record_ = SpokeConnectBuffer{};
    }
    Buffer& operator=(Buffer&& other) noexcept {
        if (this != &other) {
            release_owned();
            record_ = other.record_;
            other.record_ = SpokeConnectBuffer{};
        }
        return *this;
    }
    Buffer(const Buffer&) = delete;
    Buffer& operator=(const Buffer&) = delete;
    ~Buffer() { release_owned(); }

    /** Takes ownership of a raw record; the record transfers into the buffer. */
    [[nodiscard]] static Buffer adopt(SpokeConnectBuffer record) noexcept {
        Buffer buffer;
        buffer.record_ = record;
        return buffer;
    }

    /** Borrows the owned record for raw-C interop; the buffer keeps ownership. */
    [[nodiscard]] const SpokeConnectBuffer& get() const noexcept { return record_; }

    /** Transfers ownership of the record to the caller; the buffer becomes empty. */
    [[nodiscard]] SpokeConnectBuffer release() noexcept {
        const SpokeConnectBuffer released = record_;
        record_ = SpokeConnectBuffer{};
        return released;
    }

    [[nodiscard]] const uint8_t* data() const noexcept { return record_.data; }
    [[nodiscard]] size_t size() const noexcept { return record_.len; }
    [[nodiscard]] bool empty() const noexcept { return record_.len == 0; }

    /**
     * Borrows the payload — the binary length is authoritative and embedded NUL
     * bytes are content. The view is valid while this buffer is alive and is
     * refused on a temporary buffer, whose bytes die with it.
     */
    [[nodiscard]] std::string_view view() const& noexcept {
        if (record_.data == nullptr) return std::string_view();
        return std::string_view(reinterpret_cast<const char*>(record_.data), record_.len);
    }
    std::string_view view() && = delete;

    /** Materializes an owned copy of the payload. */
    [[nodiscard]] std::string str() const { return std::string(view()); }

  private:
    void release_owned() noexcept { spoke_connect_buffer_free(&record_); }

    SpokeConnectBuffer record_{};
};

namespace detail {

/**
 * Copies the C error record into `Error`. The RAII guard stays alive across the
 * copies, so a failing copy releases the record instead of leaking it.
 */
[[nodiscard]] inline Error take_error(int32_t status, const CErrorRecord& record) {
    const SpokeConnectError& source = record.get();
    Error error;
    error.status = status;
    error.message = text_of(source.message);
    error.code = optional_text_of(source.code);
    error.kind = optional_text_of(source.kind);
    error.wire_code = optional_text_of(source.wire_code);
    error.expected = source.expected;
    error.actual = source.actual;
    return error;
}

/**
 * Owns one call's C optional-buffer out-record. Ownership of the contained
 * buffer transfers exactly once; the source out-record is cleared as it does.
 */
class OptionalBufferRecord {
  public:
    OptionalBufferRecord() noexcept = default;
    OptionalBufferRecord(const OptionalBufferRecord&) = delete;
    OptionalBufferRecord& operator=(const OptionalBufferRecord&) = delete;
    ~OptionalBufferRecord() { spoke_connect_optional_buffer_free(&record_); }

    [[nodiscard]] SpokeConnectOptionalBuffer* out() noexcept { return &record_; }

    /**
     * `present == 0` → absent; `present == 1` → the owned buffer, which is a
     * present empty buffer when the payload length is zero.
     */
    [[nodiscard]] std::optional<Buffer> take() noexcept {
        if (record_.present == 0) return std::nullopt;
        const SpokeConnectOptionalBuffer transferred = record_;
        record_ = SpokeConnectOptionalBuffer{};
        return Buffer::adopt(transferred.value);
    }

  private:
    SpokeConnectOptionalBuffer record_{};
};

}  // namespace detail

/**
 * A fallible value: either the value or the `Error` the carrier reported. The
 * public result is move-only, so a buffer or handle can never be copied out of
 * it accidentally. A checked access on the wrong branch is a caller
 * programming error and aborts in every build configuration.
 */
template <typename T>
class Result {
  public:
    Result(Result&&) = default;
    Result& operator=(Result&&) = default;
    Result(const Result&) = delete;
    Result& operator=(const Result&) = delete;

    [[nodiscard]] static Result success(T value) {
        return Result(std::move(value), SuccessTag{});
    }
    [[nodiscard]] static Result failure(Error error) {
        return Result(std::move(error), FailureTag{});
    }

    [[nodiscard]] bool has_value() const noexcept { return storage_.index() == 0; }
    [[nodiscard]] explicit operator bool() const noexcept { return has_value(); }

    T& value() & noexcept { return *require_value(); }
    const T& value() const& noexcept { return *require_value(); }
    T&& value() && noexcept { return std::move(value()); }

    Error& error() & noexcept { return *require_error(); }
    const Error& error() const& noexcept { return *require_error(); }
    Error&& error() && noexcept { return std::move(error()); }

  private:
    struct SuccessTag {};
    struct FailureTag {};

    Result(T&& value, SuccessTag) : storage_(std::in_place_index<0>, std::move(value)) {}
    Result(Error&& error, FailureTag) : storage_(std::in_place_index<1>, std::move(error)) {}

    [[nodiscard]] T* require_value() noexcept {
        T* found = std::get_if<0>(&storage_);
        if (found == nullptr) detail::contract_violation("Result::value() on a failure");
        return found;
    }
    [[nodiscard]] const T* require_value() const noexcept {
        const T* found = std::get_if<0>(&storage_);
        if (found == nullptr) detail::contract_violation("Result::value() on a failure");
        return found;
    }
    [[nodiscard]] Error* require_error() noexcept {
        Error* found = std::get_if<1>(&storage_);
        if (found == nullptr) detail::contract_violation("Result::error() on a success");
        return found;
    }
    [[nodiscard]] const Error* require_error() const noexcept {
        const Error* found = std::get_if<1>(&storage_);
        if (found == nullptr) detail::contract_violation("Result::error() on a success");
        return found;
    }

    std::variant<T, Error> storage_;
};

/** The `Result` for an operation that reports no value. */
template <>
class Result<void> {
  public:
    Result(Result&&) = default;
    Result& operator=(Result&&) = default;
    Result(const Result&) = delete;
    Result& operator=(const Result&) = delete;

    [[nodiscard]] static Result success() noexcept { return Result(); }
    [[nodiscard]] static Result failure(Error error) { return Result(std::move(error)); }

    [[nodiscard]] bool has_value() const noexcept { return !error_.has_value(); }
    [[nodiscard]] explicit operator bool() const noexcept { return has_value(); }

    void value() const noexcept {
        if (!error_.has_value()) return;
        detail::contract_violation("Result<void>::value() on a failure");
    }

    Error& error() & noexcept { return *require_error(); }
    const Error& error() const& noexcept { return *require_error(); }
    Error&& error() && noexcept { return std::move(error()); }

  private:
    Result() noexcept = default;
    explicit Result(Error&& error) : error_(std::move(error)) {}

    [[nodiscard]] Error* require_error() noexcept {
        if (!error_.has_value()) detail::contract_violation("Result<void>::error() on a success");
        return &*error_;
    }
    [[nodiscard]] const Error* require_error() const noexcept {
        if (!error_.has_value()) detail::contract_violation("Result<void>::error() on a success");
        return &*error_;
    }

    std::optional<Error> error_;
};

/**
 * Borrows text into the C slice form. The length is authoritative — no
 * `strlen` is ever applied, so an embedded NUL is ordinary content — and an
 * empty input crosses as the NULL/zero span the boundary documents.
 */
[[nodiscard]] inline SpokeConnectSlice slice(std::string_view text) noexcept {
    SpokeConnectSlice result{};
    result.data = text.empty() ? nullptr : reinterpret_cast<const uint8_t*>(text.data());
    result.len = text.size();
    return result;
}

/**
 * Borrows raw bytes (a key, a seed, an envelope) into the C slice form, for
 * `std::array` / `std::vector` `.data()` / `.size()`. Keys stay bytes: this is
 * never a NUL-terminated string.
 */
[[nodiscard]] inline SpokeConnectSlice bytes(const uint8_t* data, size_t size) noexcept {
    SpokeConnectSlice result{};
    result.data = size == 0 ? nullptr : data;
    result.len = size;
    return result;
}

// ── Core free functions ──────────────────────────────────────────────────

/** The C boundary revision (currently 1). Not the hello protocol version. */
[[nodiscard]] inline Result<uint64_t> abi_version() {
    uint64_t version = 0;
    detail::CErrorRecord error;
    const int32_t status = spoke_connect_abi_version(&version, error.out());
    if (status != SPOKE_CONNECT_OK) {
        return Result<uint64_t>::failure(detail::take_error(status, error));
    }
    return Result<uint64_t>::success(version);
}

/** The connect protocol version exchanged in `ConnectHello` (currently 1). */
[[nodiscard]] inline Result<uint64_t> protocol_version() {
    uint64_t version = 0;
    detail::CErrorRecord error;
    const int32_t status = spoke_connect_protocol_version(&version, error.out());
    if (status != SPOKE_CONNECT_OK) {
        return Result<uint64_t>::failure(detail::take_error(status, error));
    }
    return Result<uint64_t>::success(version);
}

/**
 * Derives the wire `peer_id` for a 32-byte Ed25519 public key. The result is
 * UTF-8 text.
 */
[[nodiscard]] inline Result<Buffer> derive_peer_id_from_ed25519_pubkey(
    SpokeConnectSlice pubkey) {
    SpokeConnectBuffer out{};
    detail::CErrorRecord error;
    const int32_t status =
        spoke_connect_derive_peer_id_from_ed25519_pubkey(pubkey, &out, error.out());
    Buffer peer_id = Buffer::adopt(out);
    if (status != SPOKE_CONNECT_OK) {
        return Result<Buffer>::failure(detail::take_error(status, error));
    }
    return Result<Buffer>::success(std::move(peer_id));
}

/**
 * Signs a hello with a raw 32-byte Ed25519 secret key. `nonce` must meet the
 * wire floor and `host_json` is the canonical JSON of the host capability
 * manifest embedded in the hello; the result is the signed `ConnectHello`
 * envelope as JSON.
 */
[[nodiscard]] inline Result<Buffer> sign_hello_ed25519(SpokeConnectSlice secret,
                                                       std::string_view nonce,
                                                       std::string_view host_json) {
    SpokeConnectBuffer out{};
    detail::CErrorRecord error;
    const int32_t status = spoke_connect_sign_hello_ed25519(secret, slice(nonce),
                                                            slice(host_json), &out, error.out());
    Buffer hello = Buffer::adopt(out);
    if (status != SPOKE_CONNECT_OK) {
        return Result<Buffer>::failure(detail::take_error(status, error));
    }
    return Result<Buffer>::success(std::move(hello));
}

/**
 * Verifies a received hello against a 32-byte Ed25519 public key.
 * `expected_peer_id` is the authenticated remote peer and `hello_json` the
 * received `ConnectHello` envelope as JSON.
 */
[[nodiscard]] inline Result<void> verify_hello_ed25519(SpokeConnectSlice public_key,
                                                       std::string_view expected_peer_id,
                                                       std::string_view hello_json) {
    detail::CErrorRecord error;
    const int32_t status = spoke_connect_verify_hello_ed25519(
        public_key, slice(expected_peer_id), slice(hello_json), error.out());
    if (status != SPOKE_CONNECT_OK) {
        return Result<void>::failure(detail::take_error(status, error));
    }
    return Result<void>::success();
}

/**
 * Whether `peer_id` is on the allowlist. Fails closed: an empty allowlist
 * rejects every peer. The array is the C form — callers may hand in
 * `std::array<SpokeConnectSlice, N>::data()` with its size.
 */
[[nodiscard]] inline Result<bool> is_allowlisted(const SpokeConnectSlice* allowlist,
                                                 size_t allowlist_count,
                                                 std::string_view peer_id) {
    uint8_t allowed = 0;
    detail::CErrorRecord error;
    const int32_t status = spoke_connect_is_allowlisted(allowlist, allowlist_count,
                                                        slice(peer_id), &allowed, error.out());
    if (status != SPOKE_CONNECT_OK) {
        return Result<bool>::failure(detail::take_error(status, error));
    }
    return Result<bool>::success(allowed != 0);
}

/** Checks that a response echoes the request's session id, sequence and request id. */
[[nodiscard]] inline Result<void> check_response_correlation(
    std::string_view expected_session_id, uint64_t expected_sequence,
    std::string_view expected_request_id, std::string_view actual_session_id,
    uint64_t actual_sequence, std::string_view actual_request_id) {
    detail::CErrorRecord error;
    const int32_t status = spoke_connect_check_response_correlation(
        slice(expected_session_id), expected_sequence, slice(expected_request_id),
        slice(actual_session_id), actual_sequence, slice(actual_request_id), error.out());
    if (status != SPOKE_CONNECT_OK) {
        return Result<void>::failure(detail::take_error(status, error));
    }
    return Result<void>::success();
}

/**
 * Whether `op` may be dispatched with `negotiated_capabilities`. Fails closed:
 * an unknown `op` is not authorized by this gate.
 */
[[nodiscard]] inline Result<bool> dispatch_allowed(
    std::string_view op, const SpokeConnectSlice* negotiated_capabilities,
    size_t negotiated_count) {
    uint8_t allowed = 0;
    detail::CErrorRecord error;
    const int32_t status = spoke_connect_dispatch_allowed(
        slice(op), negotiated_capabilities, negotiated_count, &allowed, error.out());
    if (status != SPOKE_CONNECT_OK) {
        return Result<bool>::failure(detail::take_error(status, error));
    }
    return Result<bool>::success(allowed != 0);
}

/**
 * The capability required to dispatch `op`; absent for product-defined ops.
 * A present capability is a `Buffer` — `view()` borrows it, `str()` copies it.
 */
[[nodiscard]] inline Result<std::optional<Buffer>> required_capability(
    std::string_view op) {
    detail::OptionalBufferRecord out;
    detail::CErrorRecord error;
    const int32_t status = spoke_connect_required_capability(slice(op), out.out(), error.out());
    if (status != SPOKE_CONNECT_OK) {
        return Result<std::optional<Buffer>>::failure(detail::take_error(status, error));
    }
    return Result<std::optional<Buffer>>::success(out.take());
}

// ── Core session objects ─────────────────────────────────────────────────

/** Single-use `(peer_id, nonce)` replay store. */
class NonceStore
    : public detail::HandleBase<SpokeConnectNonceStore, &spoke_connect_nonce_store_free> {
  public:
    /** Creates an empty store. */
    [[nodiscard]] static Result<NonceStore> create() {
        SpokeConnectNonceStore* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_nonce_store_new(&handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<NonceStore>::failure(detail::take_error(status, error));
        }
        return Result<NonceStore>::success(NonceStore(handle));
    }

    /** Records `(peer_id, nonce)`; true on the first acceptance. */
    [[nodiscard]] Result<bool> check_and_record(std::string_view peer_id,
                                                std::string_view nonce) const {
        uint8_t recorded = 0;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_nonce_store_check_and_record(
            get(), slice(peer_id), slice(nonce), &recorded, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<bool>::failure(detail::take_error(status, error));
        }
        return Result<bool>::success(recorded != 0);
    }

  private:
    NonceStore() noexcept = default;
    explicit NonceStore(SpokeConnectNonceStore* handle) noexcept : HandleBase(handle) {}
};

/** Outbound sequence counter, starting at 0. */
class OutboundSequence : public detail::HandleBase<SpokeConnectOutboundSequence,
                                                   &spoke_connect_outbound_sequence_free> {
  public:
    [[nodiscard]] static Result<OutboundSequence> create() {
        SpokeConnectOutboundSequence* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_outbound_sequence_new(&handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<OutboundSequence>::failure(detail::take_error(status, error));
        }
        return Result<OutboundSequence>::success(OutboundSequence(handle));
    }

    /** Allocates the next outbound sequence — 0 on the first call. */
    [[nodiscard]] Result<uint64_t> allocate() const {
        uint64_t sequence = 0;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_outbound_sequence_allocate(get(), &sequence, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<uint64_t>::failure(detail::take_error(status, error));
        }
        return Result<uint64_t>::success(sequence);
    }

  private:
    OutboundSequence() noexcept = default;
    explicit OutboundSequence(SpokeConnectOutboundSequence* handle) noexcept
        : HandleBase(handle) {}
};

/** Inbound sequence expectation, starting at 0. */
class InboundSequence
    : public detail::HandleBase<SpokeConnectInboundSequence,
                                &spoke_connect_inbound_sequence_free> {
  public:
    [[nodiscard]] static Result<InboundSequence> create() {
        SpokeConnectInboundSequence* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_inbound_sequence_new(&handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<InboundSequence>::failure(detail::take_error(status, error));
        }
        return Result<InboundSequence>::success(InboundSequence(handle));
    }

    /**
     * Accepts `sequence` iff it is the next expected one and returns the
     * advanced expectation. A replayed or out-of-order sequence fails without
     * advancing it, with `expected` / `actual` on the error.
     */
    [[nodiscard]] Result<uint64_t> advance(int64_t sequence) const {
        uint64_t next_expected = 0;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_inbound_sequence_advance(get(), sequence, &next_expected, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<uint64_t>::failure(detail::take_error(status, error));
        }
        return Result<uint64_t>::success(next_expected);
    }

  private:
    InboundSequence() noexcept = default;
    explicit InboundSequence(SpokeConnectInboundSequence* handle) noexcept
        : HandleBase(handle) {}
};

// ── Loopback pair and ends ───────────────────────────────────────────────

/** One end of a back-to-back in-memory loopback connection. */
class LoopbackTransport : public detail::HandleBase<SpokeConnectLoopbackTransport,
                                                    &spoke_connect_loopback_transport_free> {
  public:
    /** Sends one envelope to the peer end. */
    [[nodiscard]] Result<void> send(std::string_view envelope) const {
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_loopback_transport_send(get(), slice(envelope), error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<void>::failure(detail::take_error(status, error));
        }
        return Result<void>::success();
    }

    /** Receives the next envelope, blocking until one arrives or the connection closes. */
    [[nodiscard]] Result<Buffer> recv() const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_loopback_transport_recv(get(), &out, error.out());
        Buffer envelope = Buffer::adopt(out);
        if (status != SPOKE_CONNECT_OK) {
            return Result<Buffer>::failure(detail::take_error(status, error));
        }
        return Result<Buffer>::success(std::move(envelope));
    }

    /** Closes the whole connection (both directions); idempotent. */
    [[nodiscard]] Result<void> close() const {
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_loopback_transport_close(get(), error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<void>::failure(detail::take_error(status, error));
        }
        return Result<void>::success();
    }

  private:
    friend class LoopbackTransportPair;

    LoopbackTransport() noexcept = default;
    explicit LoopbackTransport(SpokeConnectLoopbackTransport* handle) noexcept
        : HandleBase(handle) {}
};

/** Both ends of one back-to-back in-memory loopback connection. */
class LoopbackTransportPair
    : public detail::HandleBase<SpokeConnectLoopbackTransportPair,
                                &spoke_connect_loopback_transport_pair_free> {
  public:
    [[nodiscard]] static Result<LoopbackTransportPair> create() {
        SpokeConnectLoopbackTransportPair* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_loopback_transport_pair_new(&handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<LoopbackTransportPair>::failure(detail::take_error(status, error));
        }
        return Result<LoopbackTransportPair>::success(LoopbackTransportPair(handle));
    }

    /** Borrows the client end into a new owned end; the pair keeps its own reference. */
    [[nodiscard]] Result<LoopbackTransport> client() const {
        SpokeConnectLoopbackTransport* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_loopback_transport_pair_client(get(), &handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<LoopbackTransport>::failure(detail::take_error(status, error));
        }
        return Result<LoopbackTransport>::success(LoopbackTransport(handle));
    }

    /** Borrows the server end into a new owned end. */
    [[nodiscard]] Result<LoopbackTransport> server() const {
        SpokeConnectLoopbackTransport* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_loopback_transport_pair_server(get(), &handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<LoopbackTransport>::failure(detail::take_error(status, error));
        }
        return Result<LoopbackTransport>::success(LoopbackTransport(handle));
    }

  private:
    LoopbackTransportPair() noexcept = default;
    explicit LoopbackTransportPair(SpokeConnectLoopbackTransportPair* handle) noexcept
        : HandleBase(handle) {}
};

}  // namespace spoke::connect

#endif /* SPOKE_CONNECT_HPP */
