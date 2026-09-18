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
 *
 * A host context that the carrier calls — a transport, a ports provider, a tool
 * handler — arrives as a `std::function` record whose factory refuses an
 * incomplete record and hands ownership over only on success. In
 * exception-enabled builds the thunks contain an escaping host exception into
 * the frozen `SPOKE_CONNECT_TRANSPORT_IO` / `SPOKE_CONNECT_FFI_REJECTED` row;
 * with exceptions disabled the header contains no `try`, `catch` or `throw` at
 * all, and a host callback reports failure through its `Result`.
 */

#ifndef SPOKE_CONNECT_HPP
#define SPOKE_CONNECT_HPP

#include "spoke_connect.h"

#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <functional>
#include <memory>
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

// ── Host callback records ────────────────────────────────────────────────
//
// A bridge is three pieces: the host record below, the static thunks that
// render the complete C table, and the handle the carrier's reference lives in.
// A thunk never calls back into an operational C/C++ API and holds no lock
// across a host callback — the host context it was handed is the only thing it
// touches.

/**
 * The host transport callbacks of one connection. Every member is required: a
 * record with an unset slot is refused before the carrier sees it, so a host
 * that cannot send, receive or close never produces a half-served connection.
 *
 * The carrier copies the table, takes ownership of this record on success, and
 * runs its destructor exactly once after the last reference and in-flight
 * callback are gone. Callbacks run on the carrier's blocking pool and may run
 * concurrently, so the captured state must be thread-safe and free of thread
 * affinity. Capture by value — a reference capture must outlive the record.
 */
struct TransportCallbacks {
    /** Sends one envelope; a closed connection reports `SPOKE_CONNECT_TRANSPORT_CLOSED`. */
    std::function<Result<void>(std::string_view)> send;
    /** Receives the next envelope, blocking until one arrives or the transport closes. */
    std::function<Result<std::string>()> recv;
    /** Closes the connection; it must also unblock a pending or later `recv`. */
    std::function<Result<void>()> close;
};

/**
 * The host ports provider: one member per served ports method, named exactly as
 * the C table. Every output is the JSON text the carrier parses, and `revision`
 * follows the optional-scalar rule (`nullopt` = no expectation, `0` = a
 * present zero). A method the host does not serve must still be implemented:
 * return an application reject so an explicitly refusing provider stays
 * distinguishable from an absent `ports` handler.
 */
struct PortsCallbacks {
    std::function<Result<std::string>(std::string_view entry_id)> get_knowledge_entry;
    std::function<Result<std::string>(std::string_view entry_json,
                                      std::optional<uint64_t> expected_base_revision)>
        put_knowledge_entry;
    std::function<Result<std::string>(std::string_view relation_id)> get_relation;
    std::function<Result<std::string>(std::string_view relation_json,
                                      std::optional<uint64_t> expected_base_revision)>
        put_relation;
    std::function<Result<std::string>(std::string_view scope_json)> list_knowledge_entries;
    std::function<Result<std::string>(std::string_view scope_json)> list_timeline_events;
    std::function<Result<std::string>(std::string_view findings_json)> put_findings;
    /** The rule references cross as the borrowed C array; the host never owns it. */
    std::function<Result<std::string>(const SpokeConnectSlice* rule_refs, size_t rule_refs_count)>
        list_rules;
    std::function<Result<std::string>()> list_peer_host_capability_manifests;
    std::function<Result<std::string>(std::string_view project_request_json)> project;
    std::function<Result<std::string>(std::string_view compute_request_json)> compute;
    std::function<Result<std::string>(std::string_view scope_json)> list_fork_timeline_events;
    std::function<Result<std::string>(std::string_view extract_request_json)> extract;
};

/**
 * The host tool handler: `handle` answers the arguments JSON with the result
 * JSON. Registration is last-wins on the serving side, and a handler should
 * only report `SPOKE_CONNECT_FFI_REJECTED` — anything else is contained into
 * the carrier's `INTERNAL_ERROR` row.
 */
struct ToolCallbacks {
    std::function<Result<std::string>(std::string_view arguments_json)> handle;
};

namespace detail {

/**
 * Whether this translation unit is built with exception handling: MSVC reports
 * it through `_CPPUNWIND`, other compilers through `__cpp_exceptions` /
 * `__EXCEPTIONS`. The containment below exists only when it does — with
 * exceptions disabled the preprocessor removes every `try`, `catch` and `throw`
 * from this header, so such a translation unit contains none of the three.
 */
#if defined(_CPPUNWIND) || defined(__cpp_exceptions) || defined(__EXCEPTIONS)
#define SPOKE_CONNECT_HPP_EXCEPTION_CONTAINMENT 1
#else
#define SPOKE_CONNECT_HPP_EXCEPTION_CONTAINMENT 0
#endif

/**
 * The static row the containment path writes. Every field points at storage the
 * host already owns, because the path is reporting a failure and must not be
 * able to produce a second one.
 */
struct Containment {
    int32_t status;
    const char* message;
    const char* code;  /* nullptr = absent */
    const char* kind;  /* nullptr = absent */
};

/** A host transport callback threw — a transport I/O failure. */
inline constexpr Containment kTransportContainment{
    SPOKE_CONNECT_TRANSPORT_IO, "the host transport callback threw an exception", nullptr, nullptr};

/** A host ports or tool callback threw — the `INTERNAL_ERROR` containment row. */
inline constexpr Containment kCallbackContainment{SPOKE_CONNECT_FFI_REJECTED,
                                                  "the host callback threw an exception",
                                                  "INTERNAL_ERROR", "callback"};

/** Deletes a host payload once the carrier has copied it. */
inline void SPOKE_CONNECT_CALL release_owned_text(void* release_context, const uint8_t*,
                                                  size_t) noexcept {
    delete static_cast<std::string*>(release_context);
}

/** Releases static storage: the address stays the host's. */
inline void SPOKE_CONNECT_CALL release_static_text(void*, const uint8_t*, size_t) noexcept {}

/** One text field over storage the host already owns; the carrier's no-op
    release means nothing is ever freed. */
[[nodiscard]] inline SpokeConnectForeignBuffer static_field(const char* text) noexcept {
    SpokeConnectForeignBuffer field{};
    field.data = reinterpret_cast<const uint8_t*>(text);
    field.len = std::strlen(text);
    field.release = &release_static_text;
    return field;
}

/**
 * Writes one callback's success payload. A zero-length result uses a zero
 * record: the carrier releases only a populated buffer, so a heap holder for an
 * empty payload would leak. A non-empty payload moves into host-owned storage
 * whose `release_context` frees it once the carrier has copied the bytes.
 */
inline void write_payload(SpokeConnectForeignBuffer* out_payload, std::string&& payload) {
    if (out_payload == nullptr) return;
    *out_payload = SpokeConnectForeignBuffer{};
    if (payload.empty()) return;
    std::string* holder = new std::string(std::move(payload));
    out_payload->data = reinterpret_cast<const uint8_t*>(holder->data());
    out_payload->len = holder->size();
    out_payload->release_context = holder;
    out_payload->release = &release_owned_text;
}

/**
 * Owns the host storage one callback failure needs until the carrier has taken
 * the record. Each non-empty field moves into its own holder, because the
 * carrier runs one `release` per populated buffer: a holder for an empty field
 * would never be released, and one holder shared by several fields would be
 * freed more than once.
 *
 * Building several fields can fail part-way (an allocating build only); the
 * guard then releases what it already holds, and the caller answers with the
 * static containment row instead.
 */
class ErrorPackage {
  public:
    ErrorPackage() noexcept = default;
    ErrorPackage(const ErrorPackage&) = delete;
    ErrorPackage& operator=(const ErrorPackage&) = delete;
    ~ErrorPackage() { reset(); }

    /** Packages `error`. A field is present per its `optional`, and a present
        empty text crosses as a present empty field rather than an absent one. */
    void build(Error&& error) {
        record_.message = field(std::move(error.message), message_);
        record_.code = optional_field(std::move(error.code), code_);
        record_.kind = optional_field(std::move(error.kind), kind_);
        record_.wire_code = optional_field(std::move(error.wire_code), wire_code_);
    }

    /** Hands the record to the carrier, which now owns every holder. */
    void commit(SpokeConnectForeignError* out_error) noexcept {
        *out_error = record_;
        release_ownership();
    }

  private:
    [[nodiscard]] static SpokeConnectForeignBuffer field(std::string&& text,
                                                         std::string*& owned) {
        if (text.empty()) {
            // Present and empty: the address of a static empty string carries
            // presence, where a holder would be allocated but never released.
            return static_field("");
        }
        owned = new std::string(std::move(text));
        SpokeConnectForeignBuffer slot{};
        slot.data = reinterpret_cast<const uint8_t*>(owned->data());
        slot.len = owned->size();
        slot.release_context = owned;
        slot.release = &release_owned_text;
        return slot;
    }

    [[nodiscard]] static SpokeConnectForeignBuffer optional_field(
        std::optional<std::string>&& text, std::string*& owned) {
        if (!text.has_value()) return SpokeConnectForeignBuffer{};
        return field(std::move(*text), owned);
    }

    void release_ownership() noexcept {
        message_ = nullptr;
        code_ = nullptr;
        kind_ = nullptr;
        wire_code_ = nullptr;
        record_ = SpokeConnectForeignError{};
    }

    void reset() noexcept {
        delete message_;
        delete code_;
        delete kind_;
        delete wire_code_;
        release_ownership();
    }

    SpokeConnectForeignError record_{};
    std::string* message_ = nullptr;
    std::string* code_ = nullptr;
    std::string* kind_ = nullptr;
    std::string* wire_code_ = nullptr;
};

/** Renders one host-reported failure. A normally returned status is never
    reclassified — the carrier receives exactly what the host reported. */
inline void write_error(SpokeConnectForeignError* out_error, Error&& error) {
    ErrorPackage package;
    package.build(std::move(error));
    package.commit(out_error);
}

/** Renders the containment row from static literals. */
inline void write_containment(SpokeConnectForeignError* out_error,
                              const Containment& containment) noexcept {
    SpokeConnectForeignError record{};
    record.message = static_field(containment.message);
    if (containment.code != nullptr) record.code = static_field(containment.code);
    if (containment.kind != nullptr) record.kind = static_field(containment.kind);
    *out_error = record;
}

/** One host callback invocation's outcome. */
struct CallbackOutcome {
    bool failed = false;
    int32_t status = SPOKE_CONNECT_OK;
    bool contained = false;
    std::string payload;
    Error error;

    [[nodiscard]] static CallbackOutcome success(std::string payload = std::string()) {
        CallbackOutcome outcome;
        outcome.payload = std::move(payload);
        return outcome;
    }

    [[nodiscard]] static CallbackOutcome failure(Error error) {
        CallbackOutcome outcome;
        outcome.failed = true;
        outcome.status = error.status;
        outcome.error = std::move(error);
        return outcome;
    }

    [[nodiscard]] static CallbackOutcome containment() {
        CallbackOutcome outcome;
        outcome.contained = true;
        return outcome;
    }
};

/** Adapts a host callback's `Result` into an outcome. */
[[nodiscard]] inline CallbackOutcome outcome_of(Result<std::string>&& result) {
    if (result.has_value()) return CallbackOutcome::success(std::move(result).value());
    return CallbackOutcome::failure(std::move(result).error());
}

[[nodiscard]] inline CallbackOutcome outcome_of(Result<void>&& result) {
    if (result.has_value()) return CallbackOutcome::success();
    return CallbackOutcome::failure(std::move(result).error());
}

/**
 * Runs one host callback and renders its outcome onto the C boundary, both
 * inside the containment: exception-enabled builds convert an escaping
 * exception — from the host callback or from the packaging that follows it —
 * into the bridge's static row, while disabled builds preprocess the whole
 * `try` / `catch` away, so such a translation unit contains neither.
 *
 * A success payload moves into host-owned storage; a host-reported failure
 * packages its `Error` fields and returns the status it carried, unreclassified.
 */
template <typename Body>
[[nodiscard]] inline int32_t callback_return(const Containment& containment,
                                             SpokeConnectForeignBuffer* out_payload,
                                             SpokeConnectForeignError* out_error, Body&& body) {
    // Read here rather than only in the containment branch below: a build with
    // exceptions disabled still compiles this function without an
    // unused-parameter diagnostic.
    (void)containment;
#if SPOKE_CONNECT_HPP_EXCEPTION_CONTAINMENT
    try {
#endif
        CallbackOutcome outcome = body();
        if (outcome.failed) {
            write_error(out_error, std::move(outcome.error));
            return outcome.status;
        }
        write_payload(out_payload, std::move(outcome.payload));
        return SPOKE_CONNECT_OK;
#if SPOKE_CONNECT_HPP_EXCEPTION_CONTAINMENT
    } catch (...) {
        write_containment(out_error, containment);
        return containment.status;
    }
#endif
}

/** Borrows a callback argument span as text; the length is authoritative. */
[[nodiscard]] inline std::string_view text_view(SpokeConnectSlice span) noexcept {
    if (span.data == nullptr) return std::string_view();
    return std::string_view(reinterpret_cast<const char*>(span.data), span.len);
}

/** `present == 0` → absent; `present == 1` → the value, even when it is zero. */
[[nodiscard]] inline std::optional<uint64_t> optional_of(
    SpokeConnectOptionalU64 optional) noexcept {
    if (optional.present == 0) return std::nullopt;
    return optional.value;
}

/** The C optional-scalar form of an optional value. */
[[nodiscard]] inline SpokeConnectOptionalU64 optional_u64(
    std::optional<uint64_t> value) noexcept {
    SpokeConnectOptionalU64 optional{};
    if (value.has_value()) {
        optional.present = 1;
        optional.value = *value;
    }
    return optional;
}

/** Adopts one call's owned output buffer, or reports the failure it recorded. */
[[nodiscard]] inline Result<Buffer> buffered(int32_t status, SpokeConnectBuffer out,
                                             const CErrorRecord& error) {
    Buffer buffer = Buffer::adopt(out);
    if (status != SPOKE_CONNECT_OK) return Result<Buffer>::failure(take_error(status, error));
    return Result<Buffer>::success(std::move(buffer));
}

/** Adopts one call's optional output buffer, or reports the failure it recorded. */
[[nodiscard]] inline Result<std::optional<Buffer>> optional_buffered(
    int32_t status, OptionalBufferRecord& out, const CErrorRecord& error) {
    std::optional<Buffer> value = out.take();
    if (status != SPOKE_CONNECT_OK) {
        return Result<std::optional<Buffer>>::failure(take_error(status, error));
    }
    return Result<std::optional<Buffer>>::success(std::move(value));
}

/** The `Result<void>` of one call that reports no value. */
[[nodiscard]] inline Result<void> nothing(int32_t status, const CErrorRecord& error) {
    if (status != SPOKE_CONNECT_OK) return Result<void>::failure(take_error(status, error));
    return Result<void>::success();
}

/** The `SPOKE_CONNECT_INVALID_ARGUMENT` a missing callback record produces. */
[[nodiscard]] inline Error missing_record(const char* record) {
    Error error;
    error.status = SPOKE_CONNECT_INVALID_ARGUMENT;
    error.message = std::string(record) + " is NULL";
    return error;
}

/** The `SPOKE_CONNECT_INVALID_ARGUMENT` one unset callback slot produces. */
[[nodiscard]] inline Error unset_slot(const char* record, const char* slot) {
    Error error;
    error.status = SPOKE_CONNECT_INVALID_ARGUMENT;
    error.message = std::string(record) + "::" + slot + " is not set";
    return error;
}

/** The first required transport slot that is not set, or `nullptr` when the
    record is complete. */
[[nodiscard]] inline const char* missing_slot(const TransportCallbacks& callbacks) noexcept {
    if (!callbacks.send) return "send";
    if (!callbacks.recv) return "recv";
    if (!callbacks.close) return "close";
    return nullptr;
}

/** The first required ports slot that is not set, or `nullptr` when the record
    is complete. */
[[nodiscard]] inline const char* missing_slot(const PortsCallbacks& callbacks) noexcept {
    if (!callbacks.get_knowledge_entry) return "get_knowledge_entry";
    if (!callbacks.put_knowledge_entry) return "put_knowledge_entry";
    if (!callbacks.get_relation) return "get_relation";
    if (!callbacks.put_relation) return "put_relation";
    if (!callbacks.list_knowledge_entries) return "list_knowledge_entries";
    if (!callbacks.list_timeline_events) return "list_timeline_events";
    if (!callbacks.put_findings) return "put_findings";
    if (!callbacks.list_rules) return "list_rules";
    if (!callbacks.list_peer_host_capability_manifests) {
        return "list_peer_host_capability_manifests";
    }
    if (!callbacks.project) return "project";
    if (!callbacks.compute) return "compute";
    if (!callbacks.list_fork_timeline_events) return "list_fork_timeline_events";
    if (!callbacks.extract) return "extract";
    return nullptr;
}

/** The first required tool slot that is not set, or `nullptr` when the record
    is complete. */
[[nodiscard]] inline const char* missing_slot(const ToolCallbacks& callbacks) noexcept {
    if (!callbacks.handle) return "handle";
    return nullptr;
}

// The thunks below are the complete C callback tables rendered over the host
// records. They are the only place the carrier enters this header. They keep
// C++ linkage on purpose: a C-linkage name here would share the global C
// namespace with the host's own callback helpers, which the smoke (and any
// binding that hand-rolls a table) legitimately defines.

inline int32_t SPOKE_CONNECT_CALL transport_send(void* user_data, SpokeConnectSlice envelope,
                                                 SpokeConnectForeignError* out_error) noexcept {
    auto* callbacks = static_cast<TransportCallbacks*>(user_data);
    const std::string_view text = text_view(envelope);
    return callback_return(kTransportContainment, nullptr, out_error,
                           [&] { return outcome_of(callbacks->send(text)); });
}

inline int32_t SPOKE_CONNECT_CALL transport_recv(void* user_data,
                                                 SpokeConnectForeignBuffer* out_envelope,
                                                 SpokeConnectForeignError* out_error) noexcept {
    auto* callbacks = static_cast<TransportCallbacks*>(user_data);
    return callback_return(kTransportContainment, out_envelope, out_error,
                           [&] { return outcome_of(callbacks->recv()); });
}

inline int32_t SPOKE_CONNECT_CALL transport_close(void* user_data,
                                                  SpokeConnectForeignError* out_error) noexcept {
    auto* callbacks = static_cast<TransportCallbacks*>(user_data);
    return callback_return(kTransportContainment, nullptr, out_error,
                           [&] { return outcome_of(callbacks->close()); });
}

inline void SPOKE_CONNECT_CALL transport_destroy(void* user_data) noexcept {
    delete static_cast<TransportCallbacks*>(user_data);
}

/** Runs one ports text callback — the shape nine of the thirteen slots share. */
[[nodiscard]] inline int32_t ports_text_call(void* user_data, SpokeConnectSlice input_json,
                                             SpokeConnectForeignBuffer* out_json,
                                             SpokeConnectForeignError* out_error,
                                             const std::function<Result<std::string>(std::string_view)>
                                                 PortsCallbacks::*slot) noexcept {
    auto* callbacks = static_cast<PortsCallbacks*>(user_data);
    const std::string_view input = text_view(input_json);
    return callback_return(kCallbackContainment, out_json, out_error,
                           [&] { return outcome_of((callbacks->*slot)(input)); });
}

/** Runs one ports revision callback — the `put*` pair. */
[[nodiscard]] inline int32_t ports_revision_call(
    void* user_data, SpokeConnectSlice input_json, SpokeConnectOptionalU64 expected_base_revision,
    SpokeConnectForeignBuffer* out_json, SpokeConnectForeignError* out_error,
    const std::function<Result<std::string>(std::string_view, std::optional<uint64_t>)>
        PortsCallbacks::*slot) noexcept {
    auto* callbacks = static_cast<PortsCallbacks*>(user_data);
    const std::string_view input = text_view(input_json);
    const std::optional<uint64_t> revision = optional_of(expected_base_revision);
    return callback_return(kCallbackContainment, out_json, out_error,
                           [&] { return outcome_of((callbacks->*slot)(input, revision)); });
}

inline int32_t SPOKE_CONNECT_CALL ports_get_knowledge_entry(
    void* user_data, SpokeConnectSlice input_json, SpokeConnectForeignBuffer* out_json,
    SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error,
                           &PortsCallbacks::get_knowledge_entry);
}

inline int32_t SPOKE_CONNECT_CALL ports_put_knowledge_entry(
    void* user_data, SpokeConnectSlice input_json, SpokeConnectOptionalU64 expected_base_revision,
    SpokeConnectForeignBuffer* out_json, SpokeConnectForeignError* out_error) noexcept {
    return ports_revision_call(user_data, input_json, expected_base_revision, out_json, out_error,
                               &PortsCallbacks::put_knowledge_entry);
}

inline int32_t SPOKE_CONNECT_CALL ports_get_relation(void* user_data,
                                                     SpokeConnectSlice input_json,
                                                     SpokeConnectForeignBuffer* out_json,
                                                     SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error,
                           &PortsCallbacks::get_relation);
}

inline int32_t SPOKE_CONNECT_CALL ports_put_relation(
    void* user_data, SpokeConnectSlice input_json, SpokeConnectOptionalU64 expected_base_revision,
    SpokeConnectForeignBuffer* out_json, SpokeConnectForeignError* out_error) noexcept {
    return ports_revision_call(user_data, input_json, expected_base_revision, out_json, out_error,
                               &PortsCallbacks::put_relation);
}

inline int32_t SPOKE_CONNECT_CALL ports_list_knowledge_entries(
    void* user_data, SpokeConnectSlice input_json, SpokeConnectForeignBuffer* out_json,
    SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error,
                           &PortsCallbacks::list_knowledge_entries);
}

inline int32_t SPOKE_CONNECT_CALL ports_list_timeline_events(
    void* user_data, SpokeConnectSlice input_json, SpokeConnectForeignBuffer* out_json,
    SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error,
                           &PortsCallbacks::list_timeline_events);
}

inline int32_t SPOKE_CONNECT_CALL ports_put_findings(void* user_data,
                                                     SpokeConnectSlice input_json,
                                                     SpokeConnectForeignBuffer* out_json,
                                                     SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error,
                           &PortsCallbacks::put_findings);
}

inline int32_t SPOKE_CONNECT_CALL ports_list_rules(void* user_data,
                                                   const SpokeConnectSlice* rule_refs,
                                                   size_t rule_refs_count,
                                                   SpokeConnectForeignBuffer* out_json,
                                                   SpokeConnectForeignError* out_error) noexcept {
    auto* callbacks = static_cast<PortsCallbacks*>(user_data);
    return callback_return(
        kCallbackContainment, out_json, out_error,
        [&] { return outcome_of(callbacks->list_rules(rule_refs, rule_refs_count)); });
}

inline int32_t SPOKE_CONNECT_CALL ports_list_peer_host_capability_manifests(
    void* user_data, SpokeConnectForeignBuffer* out_json,
    SpokeConnectForeignError* out_error) noexcept {
    auto* callbacks = static_cast<PortsCallbacks*>(user_data);
    return callback_return(
        kCallbackContainment, out_json, out_error,
        [&] { return outcome_of(callbacks->list_peer_host_capability_manifests()); });
}

inline int32_t SPOKE_CONNECT_CALL ports_project(void* user_data, SpokeConnectSlice input_json,
                                                SpokeConnectForeignBuffer* out_json,
                                                SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error, &PortsCallbacks::project);
}

inline int32_t SPOKE_CONNECT_CALL ports_compute(void* user_data, SpokeConnectSlice input_json,
                                                SpokeConnectForeignBuffer* out_json,
                                                SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error, &PortsCallbacks::compute);
}

inline int32_t SPOKE_CONNECT_CALL ports_list_fork_timeline_events(
    void* user_data, SpokeConnectSlice input_json, SpokeConnectForeignBuffer* out_json,
    SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error,
                           &PortsCallbacks::list_fork_timeline_events);
}

inline int32_t SPOKE_CONNECT_CALL ports_extract(void* user_data, SpokeConnectSlice input_json,
                                                SpokeConnectForeignBuffer* out_json,
                                                SpokeConnectForeignError* out_error) noexcept {
    return ports_text_call(user_data, input_json, out_json, out_error, &PortsCallbacks::extract);
}

inline void SPOKE_CONNECT_CALL ports_destroy(void* user_data) noexcept {
    delete static_cast<PortsCallbacks*>(user_data);
}

inline int32_t SPOKE_CONNECT_CALL tool_handle(void* user_data, SpokeConnectSlice arguments_json,
                                              SpokeConnectForeignBuffer* out_json,
                                              SpokeConnectForeignError* out_error) noexcept {
    auto* callbacks = static_cast<ToolCallbacks*>(user_data);
    const std::string_view arguments = text_view(arguments_json);
    return callback_return(kCallbackContainment, out_json, out_error,
                           [&] { return outcome_of(callbacks->handle(arguments)); });
}

inline void SPOKE_CONNECT_CALL tool_destroy(void* user_data) noexcept {
    delete static_cast<ToolCallbacks*>(user_data);
}

/** The complete transport table over the static thunks. */
[[nodiscard]] inline SpokeConnectTransportTable transport_table() noexcept {
    SpokeConnectTransportTable table{};
    table.send = &transport_send;
    table.recv = &transport_recv;
    table.close = &transport_close;
    table.destroy = &transport_destroy;
    return table;
}

/** The complete ports table over the static thunks. */
[[nodiscard]] inline SpokeConnectPortsHandlerTable ports_table() noexcept {
    SpokeConnectPortsHandlerTable table{};
    table.get_knowledge_entry = &ports_get_knowledge_entry;
    table.put_knowledge_entry = &ports_put_knowledge_entry;
    table.get_relation = &ports_get_relation;
    table.put_relation = &ports_put_relation;
    table.list_knowledge_entries = &ports_list_knowledge_entries;
    table.list_timeline_events = &ports_list_timeline_events;
    table.put_findings = &ports_put_findings;
    table.list_rules = &ports_list_rules;
    table.list_peer_host_capability_manifests = &ports_list_peer_host_capability_manifests;
    table.project = &ports_project;
    table.compute = &ports_compute;
    table.list_fork_timeline_events = &ports_list_fork_timeline_events;
    table.extract = &ports_extract;
    table.destroy = &ports_destroy;
    return table;
}

/** The complete tool table over the static thunks. */
[[nodiscard]] inline SpokeConnectToolHandlerTable tool_table() noexcept {
    SpokeConnectToolHandlerTable table{};
    table.handle = &tool_handle;
    table.destroy = &tool_destroy;
    return table;
}

}  // namespace detail

// ── Callback handles ─────────────────────────────────────────────────────

/**
 * Carries one host-callback transport handle. The callback record belongs to
 * the carrier from the moment `create` succeeds — ownership moved on the
 * successful C return, and `create` clears the caller's `unique_ptr` only then.
 * The C surface exports no standalone transport `close`, so there is none here:
 * a session's close reaches the transported close through the adapter or
 * responder that was built over it, and the host keeps its own explicit close.
 */
class Transport : public detail::HandleBase<SpokeConnectTransport, &spoke_connect_transport_free> {
  public:
    /** Creates a transport over `callbacks`. An unset slot (or a missing
        record) reports `SPOKE_CONNECT_INVALID_ARGUMENT` and leaves the record
        with the caller; on success the carrier owns it and runs its destructor
        exactly once. */
    [[nodiscard]] static Result<Transport> create(std::unique_ptr<TransportCallbacks>& callbacks) {
        if (callbacks == nullptr) {
            return Result<Transport>::failure(detail::missing_record("TransportCallbacks"));
        }
        if (const char* slot = detail::missing_slot(*callbacks); slot != nullptr) {
            return Result<Transport>::failure(detail::unset_slot("TransportCallbacks", slot));
        }
        const SpokeConnectTransportTable table = detail::transport_table();
        SpokeConnectTransport* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_transport_new(&table, callbacks.get(), &handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<Transport>::failure(detail::take_error(status, error));
        }
        callbacks.release();
        return Result<Transport>::success(Transport(handle));
    }

  private:
    Transport() noexcept = default;
    explicit Transport(SpokeConnectTransport* handle) noexcept : HandleBase(handle) {}
};

/**
 * Carries one host-callback ports-handler handle. Every one of the thirteen
 * slots must be set: a provider that declines a method returns an application
 * reject from that slot, which keeps an explicitly refusing provider
 * distinguishable from an absent `ports` handler.
 */
class PortsHandler
    : public detail::HandleBase<SpokeConnectPortsHandler, &spoke_connect_ports_handler_free> {
  public:
    [[nodiscard]] static Result<PortsHandler> create(std::unique_ptr<PortsCallbacks>& callbacks) {
        if (callbacks == nullptr) {
            return Result<PortsHandler>::failure(detail::missing_record("PortsCallbacks"));
        }
        if (const char* slot = detail::missing_slot(*callbacks); slot != nullptr) {
            return Result<PortsHandler>::failure(detail::unset_slot("PortsCallbacks", slot));
        }
        const SpokeConnectPortsHandlerTable table = detail::ports_table();
        SpokeConnectPortsHandler* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_ports_handler_new(&table, callbacks.get(), &handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<PortsHandler>::failure(detail::take_error(status, error));
        }
        callbacks.release();
        return Result<PortsHandler>::success(PortsHandler(handle));
    }

  private:
    PortsHandler() noexcept = default;
    explicit PortsHandler(SpokeConnectPortsHandler* handle) noexcept : HandleBase(handle) {}
};

/** Carries one host-callback tool-handler handle, for registration on either
    face of a session (served by a responder, or reverse-served by an adapter). */
class ToolHandler
    : public detail::HandleBase<SpokeConnectToolHandler, &spoke_connect_tool_handler_free> {
  public:
    [[nodiscard]] static Result<ToolHandler> create(std::unique_ptr<ToolCallbacks>& callbacks) {
        if (callbacks == nullptr) {
            return Result<ToolHandler>::failure(detail::missing_record("ToolCallbacks"));
        }
        if (const char* slot = detail::missing_slot(*callbacks); slot != nullptr) {
            return Result<ToolHandler>::failure(detail::unset_slot("ToolCallbacks", slot));
        }
        const SpokeConnectToolHandlerTable table = detail::tool_table();
        SpokeConnectToolHandler* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_tool_handler_new(&table, callbacks.get(), &handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<ToolHandler>::failure(detail::take_error(status, error));
        }
        callbacks.release();
        return Result<ToolHandler>::success(ToolHandler(handle));
    }

  private:
    ToolHandler() noexcept = default;
    explicit ToolHandler(SpokeConnectToolHandler* handle) noexcept : HandleBase(handle) {}
};

// ── Remote adapter ───────────────────────────────────────────────────────

/**
 * The dialing side of one established peer session. `connect` blocks until the
 * session is established; the transport is borrowed and cloned internally, so
 * the caller may release its transport handle while the session lives on.
 * `close` ends the session and is distinct from destruction — a destructor only
 * frees the carrier reference.
 */
class RemoteAdapter
    : public detail::HandleBase<SpokeConnectRemoteAdapter, &spoke_connect_remote_adapter_free> {
  public:
    /** Dials `transport` and returns the established adapter. `local_seed` is
        the 32-byte Ed25519 identity seed, `remote_pubkey` the peer's 32-byte
        public key, and `invoke_timeout_ms` follows the optional-scalar rule. */
    [[nodiscard]] static Result<RemoteAdapter> connect(
        const Transport& transport, SpokeConnectSlice local_seed,
        std::string_view local_manifest_json, SpokeConnectSlice remote_pubkey,
        const SpokeConnectSlice* allowlist, size_t allowlist_count,
        std::optional<uint64_t> invoke_timeout_ms = std::nullopt) {
        SpokeConnectRemoteAdapter* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_new(
            transport.get(), local_seed, slice(local_manifest_json), remote_pubkey, allowlist,
            allowlist_count, detail::optional_u64(invoke_timeout_ms), &handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<RemoteAdapter>::failure(detail::take_error(status, error));
        }
        return Result<RemoteAdapter>::success(RemoteAdapter(handle));
    }

    /** One of `Disconnected` / `Handshaking` / `Established` / `Closed`. */
    [[nodiscard]] Result<Buffer> state() const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_state(get(), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** The session id; absent until the session is established. */
    [[nodiscard]] Result<std::optional<Buffer>> session_id() const {
        detail::OptionalBufferRecord out;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_remote_adapter_session_id(get(), out.out(), error.out());
        return detail::optional_buffered(status, out, error);
    }

    /** The remote peer id; absent until the session is established. */
    [[nodiscard]] Result<std::optional<Buffer>> remote_peer_id() const {
        detail::OptionalBufferRecord out;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_remote_adapter_remote_peer_id(get(), out.out(), error.out());
        return detail::optional_buffered(status, out, error);
    }

    /** The remote host capability manifest; absent before establish. */
    [[nodiscard]] Result<std::optional<Buffer>> remote_manifest() const {
        detail::OptionalBufferRecord out;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_remote_adapter_remote_manifest(get(), out.out(), error.out());
        return detail::optional_buffered(status, out, error);
    }

    /** The local host capability manifest as JSON. */
    [[nodiscard]] Result<Buffer> get_host_capability_manifest() const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_remote_adapter_get_host_capability_manifest(get(), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `getKnowledgeEntry`. */
    [[nodiscard]] Result<Buffer> get_knowledge_entry(std::string_view entry_id) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_get_knowledge_entry(
            get(), slice(entry_id), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `putKnowledgeEntry` with an optional base revision. */
    [[nodiscard]] Result<Buffer> put_knowledge_entry(
        std::string_view entry_json,
        std::optional<uint64_t> expected_base_revision = std::nullopt) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_put_knowledge_entry(
            get(), slice(entry_json), detail::optional_u64(expected_base_revision), &out,
            error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `getRelation`. */
    [[nodiscard]] Result<Buffer> get_relation(std::string_view relation_id) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_get_relation(
            get(), slice(relation_id), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `putRelation` with an optional base revision. */
    [[nodiscard]] Result<Buffer> put_relation(
        std::string_view relation_json,
        std::optional<uint64_t> expected_base_revision = std::nullopt) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_put_relation(
            get(), slice(relation_json), detail::optional_u64(expected_base_revision), &out,
            error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `listKnowledgeEntries`. */
    [[nodiscard]] Result<Buffer> list_knowledge_entries(std::string_view scope_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_list_knowledge_entries(
            get(), slice(scope_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `listTimelineEvents`. */
    [[nodiscard]] Result<Buffer> list_timeline_events(std::string_view scope_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_list_timeline_events(
            get(), slice(scope_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `putFindings`. */
    [[nodiscard]] Result<Buffer> put_findings(std::string_view findings_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_put_findings(
            get(), slice(findings_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `listRules` over the borrowed C array of rule refs. */
    [[nodiscard]] Result<Buffer> list_rules(const SpokeConnectSlice* rule_refs,
                                            size_t rule_refs_count) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_list_rules(
            get(), rule_refs, rule_refs_count, &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Baseline ports: `listPeerHostCapabilityManifests`. */
    [[nodiscard]] Result<Buffer> list_peer_host_capability_manifests() const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_remote_adapter_list_peer_host_capability_manifests(get(), &out,
                                                                            error.out());
        return detail::buffered(status, out, error);
    }

    /** Optional ports: `port.computable.project`. */
    [[nodiscard]] Result<Buffer> project(std::string_view project_request_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_project(
            get(), slice(project_request_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Optional ports: `port.computable.compute`. */
    [[nodiscard]] Result<Buffer> compute(std::string_view compute_request_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_compute(
            get(), slice(compute_request_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Optional ports: `port.fork.listTimelineEvents`. */
    [[nodiscard]] Result<Buffer> list_fork_timeline_events(std::string_view scope_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_list_fork_timeline_events(
            get(), slice(scope_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Core-op extract service face. */
    [[nodiscard]] Result<Buffer> extract(std::string_view extract_request_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_extract(
            get(), slice(extract_request_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Invokes a capability on the remote peer by capability id. */
    [[nodiscard]] Result<Buffer> invoke_tool(std::string_view capability_id,
                                             std::string_view arguments_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_invoke_tool(
            get(), slice(capability_id), slice(arguments_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Registers a dialer-side handler for `capability_id`, so the peer can
        invoke it in reverse. The handler is borrowed and cloned internally, so
        the wrapper may be released afterwards. */
    [[nodiscard]] Result<void> register_tool_handler(std::string_view capability_id,
                                                     const ToolHandler& handler) const {
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_register_tool_handler(
            get(), slice(capability_id), handler.get(), error.out());
        return detail::nothing(status, error);
    }

    /** Ends the session; distinct from destruction and idempotent. */
    [[nodiscard]] Result<void> close() const {
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_remote_adapter_close(get(), error.out());
        return detail::nothing(status, error);
    }

  private:
    RemoteAdapter() noexcept = default;
    explicit RemoteAdapter(SpokeConnectRemoteAdapter* handle) noexcept : HandleBase(handle) {}
};

// ── Multi-peer router ────────────────────────────────────────────────────

/**
 * Routes one op per call across the peers registered with it. The group is
 * exactly the production one — `register_peer` / `unregister_peer` /
 * `list_peers`, the composed manifest, the baseline ports and `invoke_tool`.
 * The C surface has no router `extract` / `project` / `compute` / fork member,
 * so none is invented here: the optional port families stay a per-peer adapter
 * face, reached by driving the adapter the caller already holds.
 *
 * Registration borrows and retains an adapter, so a router never owns a session
 * — destruction releases the router's own references only, and no destructor
 * closes a caller-owned adapter.
 */
class MultiPeerRouter
    : public detail::HandleBase<SpokeConnectMultiPeerRouter,
                                &spoke_connect_multi_peer_router_free> {
  public:
    /** Creates an empty router. With no peer registered every routed op is the
        terminal `no_capable_peer` reject. */
    [[nodiscard]] static Result<MultiPeerRouter> create() {
        SpokeConnectMultiPeerRouter* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_new(&handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<MultiPeerRouter>::failure(detail::take_error(status, error));
        }
        return Result<MultiPeerRouter>::success(MultiPeerRouter(handle));
    }

    /** Registers `adapter`, borrowing it and retaining an internal reference,
        and returns the peer id it was registered under. The adapter stays
        caller-owned, so a failed registration changes nothing for it. */
    [[nodiscard]] Result<Buffer> register_peer(const RemoteAdapter& adapter) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_register_peer(
            get(), adapter.get(), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Removes a peer id from the registry; the adapter itself is untouched. */
    [[nodiscard]] Result<void> unregister_peer(std::string_view peer_id) const {
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_multi_peer_router_unregister_peer(get(), slice(peer_id), error.out());
        return detail::nothing(status, error);
    }

    /** The registered peer ids, in registration order, as a JSON array. */
    [[nodiscard]] Result<Buffer> list_peers() const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_multi_peer_router_list_peers(get(), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** The composed host capability manifest of the registered peers. */
    [[nodiscard]] Result<Buffer> get_host_capability_manifest() const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_get_host_capability_manifest(
            get(), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `getKnowledgeEntry`. */
    [[nodiscard]] Result<Buffer> get_knowledge_entry(std::string_view entry_id) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_get_knowledge_entry(
            get(), slice(entry_id), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `putKnowledgeEntry` with an optional base revision. */
    [[nodiscard]] Result<Buffer> put_knowledge_entry(
        std::string_view entry_json,
        std::optional<uint64_t> expected_base_revision = std::nullopt) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_put_knowledge_entry(
            get(), slice(entry_json), detail::optional_u64(expected_base_revision), &out,
            error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `getRelation`. */
    [[nodiscard]] Result<Buffer> get_relation(std::string_view relation_id) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_get_relation(
            get(), slice(relation_id), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `putRelation` with an optional base revision. */
    [[nodiscard]] Result<Buffer> put_relation(
        std::string_view relation_json,
        std::optional<uint64_t> expected_base_revision = std::nullopt) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_put_relation(
            get(), slice(relation_json), detail::optional_u64(expected_base_revision), &out,
            error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `listKnowledgeEntries`. */
    [[nodiscard]] Result<Buffer> list_knowledge_entries(std::string_view scope_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_list_knowledge_entries(
            get(), slice(scope_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `listTimelineEvents`. */
    [[nodiscard]] Result<Buffer> list_timeline_events(std::string_view scope_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_list_timeline_events(
            get(), slice(scope_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `putFindings`. */
    [[nodiscard]] Result<Buffer> put_findings(std::string_view findings_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_put_findings(
            get(), slice(findings_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `listRules` over the borrowed C array of rule refs. */
    [[nodiscard]] Result<Buffer> list_rules(const SpokeConnectSlice* rule_refs,
                                            size_t rule_refs_count) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_list_rules(
            get(), rule_refs, rule_refs_count, &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Routed baseline ports: `listPeerHostCapabilityManifests`. */
    [[nodiscard]] Result<Buffer> list_peer_host_capability_manifests() const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_multi_peer_router_list_peer_host_capability_manifests(get(), &out,
                                                                               error.out());
        return detail::buffered(status, out, error);
    }

    /** Routes a `tools.<namespace>.<tool_id>` invoke to the registered peer
        whose cached manifest advertises that capability. With no capable peer
        the call fails with the terminal `no_capable_peer` reject and sends
        nothing. */
    [[nodiscard]] Result<Buffer> invoke_tool(std::string_view capability_id,
                                             std::string_view arguments_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_multi_peer_router_invoke_tool(
            get(), slice(capability_id), slice(arguments_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

  private:
    MultiPeerRouter() noexcept = default;
    explicit MultiPeerRouter(SpokeConnectMultiPeerRouter* handle) noexcept : HandleBase(handle) {}
};

// ── Connect responder ────────────────────────────────────────────────────

/**
 * The serving side of one accepted session. `serve` returns before the dialer
 * hello arrives; the session reaches `Established` asynchronously. The
 * transport is borrowed and cloned internally. `close` ends the session and is
 * distinct from destruction.
 */
class ConnectResponder
    : public detail::HandleBase<SpokeConnectResponder, &spoke_connect_responder_free> {
  public:
    /** Serves an accepted connection on `transport`. `peer_keys` is the peer id
        to 32-byte public key table, `ports` an optional ports provider (NULL
        serves no ports callbacks), and `invoke_timeout_ms` follows the
        optional-scalar rule. */
    [[nodiscard]] static Result<ConnectResponder> serve(
        const Transport& transport, SpokeConnectSlice local_seed,
        std::string_view local_manifest_json, const SpokeConnectSlice* allowlist,
        size_t allowlist_count, const SpokeConnectPeerKey* peer_keys, size_t peer_key_count,
        const PortsHandler* ports, std::optional<uint64_t> invoke_timeout_ms = std::nullopt) {
        SpokeConnectResponder* handle = nullptr;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_responder_new(
            transport.get(), local_seed, slice(local_manifest_json), allowlist, allowlist_count,
            peer_keys, peer_key_count, ports == nullptr ? nullptr : ports->get(),
            detail::optional_u64(invoke_timeout_ms), &handle, error.out());
        if (status != SPOKE_CONNECT_OK) {
            return Result<ConnectResponder>::failure(detail::take_error(status, error));
        }
        return Result<ConnectResponder>::success(ConnectResponder(handle));
    }

    /** One of `Disconnected` / `Handshaking` / `Established` / `Closed`. */
    [[nodiscard]] Result<Buffer> state() const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_responder_state(get(), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** The session id; absent until the session is established. */
    [[nodiscard]] Result<std::optional<Buffer>> session_id() const {
        detail::OptionalBufferRecord out;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_responder_session_id(get(), out.out(), error.out());
        return detail::optional_buffered(status, out, error);
    }

    /** The dialer's peer id; absent until the session is established. */
    [[nodiscard]] Result<std::optional<Buffer>> remote_peer_id() const {
        detail::OptionalBufferRecord out;
        detail::CErrorRecord error;
        const int32_t status =
            spoke_connect_responder_remote_peer_id(get(), out.out(), error.out());
        return detail::optional_buffered(status, out, error);
    }

    /** The dialer's host capability manifest; absent before establish. */
    [[nodiscard]] Result<std::optional<Buffer>> remote_manifest() const {
        detail::OptionalBufferRecord out;
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_responder_remote_manifest(get(), out.out(), error.out());
        return detail::optional_buffered(status, out, error);
    }

    /** Registers a served tool for `capability_id`; registration is
        last-wins. The handler is borrowed and cloned internally. */
    [[nodiscard]] Result<void> register_tool_handler(std::string_view capability_id,
                                                     const ToolHandler& handler) const {
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_responder_register_tool_handler(
            get(), slice(capability_id), handler.get(), error.out());
        return detail::nothing(status, error);
    }

    /** Invokes the peer's tool for `capability_id` from the serving side. */
    [[nodiscard]] Result<Buffer> invoke_tool(std::string_view capability_id,
                                             std::string_view arguments_json) const {
        SpokeConnectBuffer out{};
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_responder_invoke_tool(
            get(), slice(capability_id), slice(arguments_json), &out, error.out());
        return detail::buffered(status, out, error);
    }

    /** Ends the session; distinct from destruction and idempotent. */
    [[nodiscard]] Result<void> close() const {
        detail::CErrorRecord error;
        const int32_t status = spoke_connect_responder_close(get(), error.out());
        return detail::nothing(status, error);
    }

  private:
    ConnectResponder() noexcept = default;
    explicit ConnectResponder(SpokeConnectResponder* handle) noexcept : HandleBase(handle) {}
};

}  // namespace spoke::connect

// The containment branch selector is this header's own: it does not outlive the
// include, so no consumer can depend on it.
#undef SPOKE_CONNECT_HPP_EXCEPTION_CONTAINMENT

#endif /* SPOKE_CONNECT_HPP */
