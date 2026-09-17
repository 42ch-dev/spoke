/*
 * spoke-connect C ABI — hand-written contract for the `spoke-connect-capi`
 * carrier library (`libspoke_connect_capi.dylib` / `spoke_connect_capi.dll`).
 *
 * Language floor: C99 (fixed-width integers from <stdint.h>, lengths from
 * <stddef.h>). C++ inclusion uses `extern "C"`; calling into this ABI
 * requires neither exception handling nor RTTI.
 *
 * The header is owned together with the Rust exports. It is not generated:
 * `tooling/connect/cpp-symbol-check.mjs` is the executable gate that keeps
 * the declarations and the exported symbols in sync.
 *
 * ── Boundary conventions ─────────────────────────────────────────────────
 *
 * Status and results
 *   Every operation returns an `int32_t` status, puts its result in
 *   caller-supplied out parameters and accepts a `SpokeConnectError *`
 *   record. Release functions alone return `void` and have no error output.
 *   The status values are the `SPOKE_CONNECT_*` macros below; they are
 *   `int32_t` constants, never enum-typed parameters.
 *
 * Out parameters
 *   Every non-release out pointer is mandatory, writable, non-aliasing and
 *   initialized to empty/zero before work starts. A NULL out pointer, or a
 *   NULL/non-zero input span, returns `SPOKE_CONNECT_INVALID_ARGUMENT`; when
 *   `out_error` itself is NULL that status is returned without an error
 *   record. A failed call leaves no result ownership with the caller, and
 *   allocated intermediates are reclaimed.
 *
 * Scalars
 *   Booleans and presence flags use `uint8_t` and are only ever `0` or `1`.
 *   Unsigned values use `uint64_t`; signed sequence inputs use `int64_t`.
 *   Counts use `size_t` and are validated before slices are constructed.
 *
 * ABI version
 *   `spoke_connect_abi_version` reports ABI revision `1`. It is distinct from
 *   the connect hello protocol version reported by
 *   `spoke_connect_protocol_version`.
 */

#ifndef SPOKE_CONNECT_H
#define SPOKE_CONNECT_H

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32)
#  if defined(SPOKE_CONNECT_CARRIER_BUILD)
/* Building the carrier itself: plain declaration, no import decoration. */
#    define SPOKE_CONNECT_API
#  else
/* Consumer: the symbols live in the carrier's import library. */
#    define SPOKE_CONNECT_API __declspec(dllimport)
#  endif
#  define SPOKE_CONNECT_CALL __cdecl
#else
/* Default C calling convention plus explicit public symbol visibility. */
#  define SPOKE_CONNECT_API __attribute__((visibility("default")))
#  define SPOKE_CONNECT_CALL
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* ── Status values ─────────────────────────────────────────────────────── */

/** Call succeeded. */
#define SPOKE_CONNECT_OK 0
/**
 * C-boundary validation failure: a NULL out pointer, a NULL/non-zero input
 * span, a length that does not fit `isize::MAX`, non-UTF-8 text, a key that
 * is not exactly 32 bytes, a duplicate peer id or an unsupported `present`
 * flag. Local to the boundary — no connect rule was evaluated.
 */
#define SPOKE_CONNECT_INVALID_ARGUMENT 1
/** A caught panic inside the wrapper. Local to the boundary. */
#define SPOKE_CONNECT_PANIC 2

/** Core hello-gate / identity failures. */
#define SPOKE_CONNECT_INVALID_HELLO_SIGNATURE 100
/** Core error: the `(peer_id, nonce)` pair was already accepted. */
#define SPOKE_CONNECT_NONCE_REPLAY 101
/** Core error: handshake-level failure; `reason` message preserved. */
#define SPOKE_CONNECT_HANDSHAKE_FAILED 102
/** Core error: the hello nonce violates the wire constraints. */
#define SPOKE_CONNECT_INVALID_NONCE 103
/** Core error: cryptography-level failure (key bytes, base64 decoding, …). */
#define SPOKE_CONNECT_CRYPTO 104
/** Core error: RFC 8785 JCS canonicalization of the signed object failed. */
#define SPOKE_CONNECT_JCS 105
/** Core error: a capability-token proof failed validation. */
#define SPOKE_CONNECT_TOKEN_INVALID 106
/** Core error: the hello `protocol_version` is not the core protocol version. */
#define SPOKE_CONNECT_PROTOCOL_VERSION_MISMATCH 107

/** Core invoke error: the outbound sequence space (2^53-1) is exhausted. */
#define SPOKE_CONNECT_SEQUENCE_EXHAUSTED 200
/**
 * Core invoke error: an inbound `sequence` is not the next expected one.
 * `expected` / `actual` carry both numbers.
 */
#define SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH 201
/** Core invoke error: a response did not echo the request's three fields. */
#define SPOKE_CONNECT_CORRELATION_MISMATCH 202

/**
 * Dial failure before any session exists; `kind` carries the dial kind
 * (`config` / `handshake` / `transport` / `timeout` / …) and `message` the
 * detail.
 */
#define SPOKE_CONNECT_FFI_DIAL 300
/**
 * Invoke-path application rejection: `code` carries the reject code and
 * `kind` / `wire_code` are populated where the facade exposes them. Reject
 * codes are strings, not a second closed C enum.
 */
#define SPOKE_CONNECT_FFI_REJECTED 301
/** The foreign callback `Transport` is closed. */
#define SPOKE_CONNECT_TRANSPORT_CLOSED 400
/**
 * Transport-level I/O failure, including a contained foreign-callback fault.
 */
#define SPOKE_CONNECT_TRANSPORT_IO 401

/* ── Value types ───────────────────────────────────────────────────────── */

/**
 * Borrowed byte/text input: `data` is valid for the duration of the call.
 *
 * `NULL` is legal only with `len == 0`; `len` must fit `isize::MAX`. Text is
 * validated as UTF-8 and the length is authoritative — no `strlen` is ever
 * applied, so embedded NUL bytes are ordinary content for byte inputs. Keys
 * are raw bytes and are checked to exactly 32 bytes by the underlying facade.
 */
typedef struct SpokeConnectSlice {
    const uint8_t *data;
    size_t len;
} SpokeConnectSlice;

/**
 * Owned output buffer: `len` payload bytes plus a trailing zero byte that the
 * library writes and excludes from `len` (also for binary output).
 *
 * The consumer reads but never mutates the fields or the data, and never
 * frees it with its own allocator. Release with `spoke_connect_buffer_free`,
 * which deallocates inside the producing library and zeroes the struct; a
 * zero buffer (NULL `data`) is a no-op.
 */
typedef struct SpokeConnectBuffer {
    uint8_t *data;
    size_t len;
} SpokeConnectBuffer;

/**
 * Optional owned buffer: `present == 0` means absent and the value is zero;
 * `present == 1` means the contained buffer is owned, including a present
 * empty string (non-NULL `data`, `len == 0`). Any other `present` value is
 * invalid input. Release the contained buffer with
 * `spoke_connect_optional_buffer_free`, or `spoke_connect_buffer_free` on
 * `value` when `present == 1`.
 */
typedef struct SpokeConnectOptionalBuffer {
    uint8_t present;
    SpokeConnectBuffer value;
} SpokeConnectOptionalBuffer;

/**
 * Optional unsigned scalar: `present == 0` means absent and the value is
 * zero; `present == 1` means `value` is meaningful even when it is zero. Any
 * other `present` value is invalid input. An optional ports handle uses a
 * NULL pointer for absence instead.
 */
typedef struct SpokeConnectOptionalU64 {
    uint8_t present;
    uint64_t value;
} SpokeConnectOptionalU64;

/**
 * Buffer returned by a foreign callback: `data` is host-owned and valid until
 * the release function runs.
 *
 * Rust copies the bytes and then calls `release` exactly once with
 * `release_context` and the same pointer/length. A release runs only for a
 * populated buffer: a zero buffer — NULL `data` or zero `len` — is empty and
 * needs no release, and a host that returns static storage supplies a no-op
 * release function. Rust never frees host memory
 * with the host allocator.
 */
typedef struct SpokeConnectForeignBuffer {
    const uint8_t *data;
    size_t len;
    void *release_context;
    void (SPOKE_CONNECT_CALL *release)(void *release_context, const uint8_t *data, size_t len);
} SpokeConnectForeignBuffer;

/**
 * Failure record a foreign callback writes: the same four textual fields as
 * `SpokeConnectError`, carried as foreign buffers so the host keeps ownership
 * of their memory. Zero/NULL means the field is absent.
 */
typedef struct SpokeConnectForeignError {
    SpokeConnectForeignBuffer message;
    SpokeConnectForeignBuffer code;
    SpokeConnectForeignBuffer kind;
    SpokeConnectForeignBuffer wire_code;
} SpokeConnectForeignError;

/**
 * Failure out-record. The caller supplies a zeroed struct; the library
 * overwrites the fields it fills.
 *
 * `message` is owned; `code` / `kind` / `wire_code` are owned where the
 * underlying error carries them and NULL/zero otherwise. A NULL/zero `kind`
 * or `wire_code` buffer means absent; a present empty string has non-NULL
 * `data` with `len == 0`. `expected` / `actual` carry an inbound sequence
 * mismatch. Free with `spoke_connect_error_free`, which releases every field
 * and zeroes the struct; free a previous record before reusing it.
 *
 * Core errors preserve their message/reason; an inbound sequence mismatch
 * preserves both numeric fields; a dial error preserves `kind` and
 * `message`; a rejection preserves `code`, `message` and the optional `kind`
 * / `wire_code`.
 */
typedef struct SpokeConnectError {
    SpokeConnectBuffer message;
    SpokeConnectBuffer code;
    SpokeConnectBuffer kind;
    SpokeConnectBuffer wire_code;
    uint64_t expected;
    int64_t actual;
} SpokeConnectError;

/**
 * One peer key entry: the peer id it belongs to plus its 32-byte Ed25519
 * public key. Both slices are borrowed for the duration of the call.
 * Duplicate peer ids reject as invalid input rather than silently choosing
 * one key.
 */
typedef struct SpokeConnectPeerKey {
    SpokeConnectSlice peer_id;
    SpokeConnectSlice public_key;
} SpokeConnectPeerKey;

/* ── Foreign callback types ────────────────────────────────────────────── */

/**
 * Send one envelope. `envelope` is borrowed for the duration of the call; the
 * host copies anything it retains. Returns `SPOKE_CONNECT_OK`, or
 * `SPOKE_CONNECT_TRANSPORT_CLOSED` / `SPOKE_CONNECT_TRANSPORT_IO` with an
 * error record.
 */
typedef int32_t (SPOKE_CONNECT_CALL *SpokeConnectTransportSendFn)(
    void *user_data,
    SpokeConnectSlice envelope,
    SpokeConnectForeignError *out_error);

/**
 * Receive the next inbound envelope, blocking until one arrives or the
 * transport closes. The returned buffer transfers to Rust on return, which
 * copies it and then calls its release exactly once; a zero buffer needs no
 * release. Returns `SPOKE_CONNECT_OK`, or
 * `SPOKE_CONNECT_TRANSPORT_CLOSED` / `SPOKE_CONNECT_TRANSPORT_IO` with an
 * error record.
 */
typedef int32_t (SPOKE_CONNECT_CALL *SpokeConnectTransportRecvFn)(
    void *user_data,
    SpokeConnectForeignBuffer *out_envelope,
    SpokeConnectForeignError *out_error);

/**
 * Release the transport's resources. Idempotent: it must also unblock a
 * pending or later `recv` with transport-closed, and must be safe to run
 * concurrently with `recv` and with a concurrent `close`. Returns
 * `SPOKE_CONNECT_OK`, or a transport status with an error record.
 */
typedef int32_t (SPOKE_CONNECT_CALL *SpokeConnectTransportCloseFn)(
    void *user_data,
    SpokeConnectForeignError *out_error);

/**
 * Destroy a callback context. Runs exactly once, after the last Rust
 * reference and in-flight callback are gone; must not unwind.
 */
typedef void (SPOKE_CONNECT_CALL *SpokeConnectTransportDestroyFn)(void *user_data);

/** Destroys a ports/tool callback context. Same rule as the transport one. */
typedef void (SPOKE_CONNECT_CALL *SpokeConnectCallbackDestroyFn)(void *user_data);

/**
 * A ports callback that takes one borrowed UTF-8 argument and answers one
 * JSON buffer: `get_knowledge_entry`, `get_relation`,
 * `list_knowledge_entries`, `list_timeline_events`, `put_findings`,
 * `project`, `compute`, `list_fork_timeline_events` and `extract`.
 *
 * An empty argument crosses as a NULL/zero span. The argument is borrowed
 * only until the callback returns, and the returned buffer transfers to Rust
 * on return — on success as well as on error — which copies/validates it and
 * then calls its release exactly once.
 */
typedef int32_t (SPOKE_CONNECT_CALL *SpokeConnectPortsTextFn)(
    void *user_data,
    SpokeConnectSlice input_json,
    SpokeConnectForeignBuffer *out_json,
    SpokeConnectForeignError *out_error);

/**
 * A ports callback that takes one borrowed JSON argument and an optional
 * expected base revision, and answers one JSON buffer:
 * `put_knowledge_entry` and `put_relation`. `expected_base_revision` follows
 * the optional-scalar rule: `present == 0` means no expectation, `present ==
 * 1` keeps `value` even when it is zero.
 */
typedef int32_t (SPOKE_CONNECT_CALL *SpokeConnectPortsRevisionFn)(
    void *user_data,
    SpokeConnectSlice input_json,
    SpokeConnectOptionalU64 expected_base_revision,
    SpokeConnectForeignBuffer *out_json,
    SpokeConnectForeignError *out_error);

/**
 * `list_rules`: the rule references cross as an array of borrowed UTF-8
 * slices (pointer + count; a NULL pointer is legal only with count zero).
 */
typedef int32_t (SPOKE_CONNECT_CALL *SpokeConnectPortsListRulesFn)(
    void *user_data,
    const SpokeConnectSlice *rule_refs,
    size_t rule_refs_count,
    SpokeConnectForeignBuffer *out_json,
    SpokeConnectForeignError *out_error);

/**
 * A ports callback that takes no argument and answers one JSON buffer:
 * `list_peer_host_capability_manifests`.
 */
typedef int32_t (SPOKE_CONNECT_CALL *SpokeConnectPortsNoInputFn)(
    void *user_data,
    SpokeConnectForeignBuffer *out_json,
    SpokeConnectForeignError *out_error);

/**
 * The tool callback: `handle`. The arguments JSON is borrowed for the
 * duration of the call; the returned buffer transfers to Rust on return and
 * is released exactly once.
 */
typedef int32_t (SPOKE_CONNECT_CALL *SpokeConnectToolHandleFn)(
    void *user_data,
    SpokeConnectSlice arguments_json,
    SpokeConnectForeignBuffer *out_json,
    SpokeConnectForeignError *out_error);

/* ── Foreign callback tables ───────────────────────────────────────────── */

/**
 * Transport callback table. Every pointer is required; a table missing one is
 * rejected as invalid argument, ownership of `user_data` stays with the
 * caller and `destroy` is not called.
 *
 * On a successful `spoke_connect_transport_new` ownership of `user_data`
 * transfers to the adapter and its `destroy` runs exactly once, after the
 * last Rust reference and in-flight callback are gone. Callbacks run on the
 * library's blocking pool and may run concurrently: the context, its
 * callbacks and `destroy` must be thread-safe and need no thread affinity.
 */
typedef struct SpokeConnectTransportTable {
    SpokeConnectTransportSendFn send;
    SpokeConnectTransportRecvFn recv;
    SpokeConnectTransportCloseFn close;
    SpokeConnectTransportDestroyFn destroy;
} SpokeConnectTransportTable;

/**
 * Ports callback table: one pointer per `PortsHandler` method plus the
 * mandatory `destroy`. Every pointer is required; a table missing one is
 * rejected as invalid argument, ownership of `user_data` stays with the
 * caller and `destroy` is not called. A provider declines an unsupported
 * method by returning `SPOKE_CONNECT_FFI_REJECTED`, which preserves the
 * distinction between absent ports and an explicitly refusing callback.
 */
typedef struct SpokeConnectPortsHandlerTable {
    SpokeConnectPortsTextFn get_knowledge_entry;
    SpokeConnectPortsRevisionFn put_knowledge_entry;
    SpokeConnectPortsTextFn get_relation;
    SpokeConnectPortsRevisionFn put_relation;
    SpokeConnectPortsTextFn list_knowledge_entries;
    SpokeConnectPortsTextFn list_timeline_events;
    SpokeConnectPortsTextFn put_findings;
    SpokeConnectPortsListRulesFn list_rules;
    SpokeConnectPortsNoInputFn list_peer_host_capability_manifests;
    SpokeConnectPortsTextFn project;
    SpokeConnectPortsTextFn compute;
    SpokeConnectPortsTextFn list_fork_timeline_events;
    SpokeConnectPortsTextFn extract;
    SpokeConnectCallbackDestroyFn destroy;
} SpokeConnectPortsHandlerTable;

/**
 * Tool callback table: the `handle` pointer plus the mandatory `destroy`.
 * Both pointers are required; the same ownership rule as the transport table
 * applies, and a handler should only report `SPOKE_CONNECT_FFI_REJECTED`.
 */
typedef struct SpokeConnectToolHandlerTable {
    SpokeConnectToolHandleFn handle;
    SpokeConnectCallbackDestroyFn destroy;
} SpokeConnectToolHandlerTable;

/* ── Opaque object handles ─────────────────────────────────────────────── */

/**
 * Opaque object handles. Each is an incomplete, separately named struct: a
 * pointer to one owns a single boxed Rust handle (an `Arc` of the facade
 * object; a loopback pair may own its pair value).
 *
 * Methods borrow the handle. A handle returned through an out parameter is
 * owned by the caller and released exactly once with the matching `*_free`;
 * a NULL free is a no-op. Passing a handle to a router or constructor borrows
 * it and clones the Rust ownership internally — it never consumes the
 * caller's handle, so no retain API is needed. `close` is distinct from
 * `free`: hosts close adapters/responders/transports before releasing their
 * ownership. A handle must not be freed while a call using it is in flight,
 * and handles never move between independently loaded copies of the library.
 */
typedef struct SpokeConnectNonceStore SpokeConnectNonceStore;
typedef struct SpokeConnectOutboundSequence SpokeConnectOutboundSequence;
typedef struct SpokeConnectInboundSequence SpokeConnectInboundSequence;
typedef struct SpokeConnectTransport SpokeConnectTransport;
typedef struct SpokeConnectLoopbackTransport SpokeConnectLoopbackTransport;
typedef struct SpokeConnectLoopbackTransportPair SpokeConnectLoopbackTransportPair;
typedef struct SpokeConnectRemoteAdapter SpokeConnectRemoteAdapter;
typedef struct SpokeConnectMultiPeerRouter SpokeConnectMultiPeerRouter;
typedef struct SpokeConnectResponder SpokeConnectResponder;
typedef struct SpokeConnectPortsHandler SpokeConnectPortsHandler;
typedef struct SpokeConnectToolHandler SpokeConnectToolHandler;

/* ── Threading, containment and re-entrancy ────────────────────────────── */

/*
 * Calls block the calling host OS thread; concurrent invokes use concurrent
 * host threads. Foreign callbacks run on the library's Tokio blocking pool
 * and may run concurrently. A `recv` blocks for one envelope, and an
 * idempotent `close` must unblock a pending or later `recv` with
 * transport-closed — a transport implementation must allow close while recv
 * waits.
 *
 * Callbacks must not synchronously reenter operational C faces, including the
 * exported loopback helpers. Keep the carrier and host callback code loaded
 * for the process lifetime and close resources before host shutdown; no
 * native-library hot-unload guarantee is implied.
 *
 * Every exported entry contains unwinding before it crosses C, including
 * conversion and handle disposal: a core/wrapper panic becomes
 * `SPOKE_CONNECT_PANIC`, and facade-mapped invoke panics retain
 * `INTERNAL_ERROR` / `kind = panic`. Release functions contain a destructor
 * panic and return without unwinding; they do not promise recovery from
 * arbitrary memory corruption.
 *
 * Foreign functions, including release/destroy callbacks, must contain their
 * own exceptions and panics: a C ABI cannot recover from a C++ exception
 * crossing it, a process abort or an access violation.
 *
 * Rust cannot validate forged, dangling or concurrently freed non-NULL
 * pointers: those are caller contract violations, not recoverable errors.
 */

/* SPOKE_CONNECT_DECLARATIONS_BEGIN */
/* Do not add conditional declarations inside this block: the language floor is
 * a plain prototype list so `tooling/connect/cpp-symbol-check.mjs` can parse
 * it. Platform macro definitions above may be conditional. */

/** Reports the C boundary revision (currently 1). Not the hello version. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_abi_version(
    uint64_t *out_version,
    SpokeConnectError *out_error);

/** Releases an owned buffer produced by this library and zeroes the struct. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_buffer_free(
    SpokeConnectBuffer *buffer);

/** Releases the buffer contained in an optional buffer and marks it absent. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_optional_buffer_free(
    SpokeConnectOptionalBuffer *optional);

/** Releases every field of an error record and zeroes the struct. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_error_free(
    SpokeConnectError *error);

/* ── Session core: identity and hello ──────────────────────────────────── */

/**
 * Derives the wire `peer_id` string for a 32-byte Ed25519 public key. The
 * result is written to `out_peer_id` as a UTF-8 owned buffer.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_derive_peer_id_from_ed25519_pubkey(
    SpokeConnectSlice pubkey,
    SpokeConnectBuffer *out_peer_id,
    SpokeConnectError *out_error);

/**
 * Signs a hello with a raw 32-byte Ed25519 secret key and returns the signed
 * `ConnectHello` envelope as JSON in `out_hello_json`. `nonce` must meet the
 * wire floor; `host_json` is the canonical JSON of the host capability
 * manifest embedded in the hello.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_sign_hello_ed25519(
    SpokeConnectSlice secret,
    SpokeConnectSlice nonce,
    SpokeConnectSlice host_json,
    SpokeConnectBuffer *out_hello_json,
    SpokeConnectError *out_error);

/**
 * Verifies a received hello against a 32-byte Ed25519 public key.
 * `expected_peer_id` is the authenticated remote peer; `hello_json` is the
 * JSON string of the received `ConnectHello` envelope.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_verify_hello_ed25519(
    SpokeConnectSlice public_key,
    SpokeConnectSlice expected_peer_id,
    SpokeConnectSlice hello_json,
    SpokeConnectError *out_error);

/**
 * Whether `peer_id` is on the allowlist, written to `out_allowed` as 0 or 1.
 * Fails closed: an empty allowlist rejects every peer.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_is_allowlisted(
    const SpokeConnectSlice *allowlist,
    size_t allowlist_count,
    SpokeConnectSlice peer_id,
    uint8_t *out_allowed,
    SpokeConnectError *out_error);

/**
 * Checks that a response echoes the request's session id, sequence and
 * request id.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_check_response_correlation(
    SpokeConnectSlice expected_session_id,
    uint64_t expected_sequence,
    SpokeConnectSlice expected_request_id,
    SpokeConnectSlice actual_session_id,
    uint64_t actual_sequence,
    SpokeConnectSlice actual_request_id,
    SpokeConnectError *out_error);

/**
 * Whether `op` may be dispatched with `negotiated_capabilities`, written to
 * `out_allowed` as 0 or 1. Fails closed: an unknown `op` is not authorized by
 * this gate.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_dispatch_allowed(
    SpokeConnectSlice op,
    const SpokeConnectSlice *negotiated_capabilities,
    size_t negotiated_count,
    uint8_t *out_allowed,
    SpokeConnectError *out_error);

/**
 * The capability required to dispatch `op`; `out_capability` is absent
 * (`present == 0`) for product-defined ops.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_required_capability(
    SpokeConnectSlice op,
    SpokeConnectOptionalBuffer *out_capability,
    SpokeConnectError *out_error);

/** The connect protocol version exchanged in `ConnectHello` (currently 1). */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_protocol_version(
    uint64_t *out_version,
    SpokeConnectError *out_error);

/* ── Session core objects ──────────────────────────────────────────────── */

/** Creates a single-use `(peer_id, nonce)` replay store. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_nonce_store_new(
    SpokeConnectNonceStore **out_handle,
    SpokeConnectError *out_error);

/**
 * Records `(peer_id, nonce)` if it is new. `out_recorded` is 1 on the first
 * acceptance and 0 when the pair was already present.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_nonce_store_check_and_record(
    const SpokeConnectNonceStore *handle,
    SpokeConnectSlice peer_id,
    SpokeConnectSlice nonce,
    uint8_t *out_recorded,
    SpokeConnectError *out_error);

/** Releases a nonce store. A NULL handle is a no-op. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_nonce_store_free(
    SpokeConnectNonceStore *handle);

/** Creates an outbound sequence counter starting at 0. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_outbound_sequence_new(
    SpokeConnectOutboundSequence **out_handle,
    SpokeConnectError *out_error);

/** Allocates the next outbound sequence into `out_sequence`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_outbound_sequence_allocate(
    const SpokeConnectOutboundSequence *handle,
    uint64_t *out_sequence,
    SpokeConnectError *out_error);

/** Releases an outbound sequence counter. A NULL handle is a no-op. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_outbound_sequence_free(
    SpokeConnectOutboundSequence *handle);

/** Creates an inbound sequence expectation starting at 0. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_inbound_sequence_new(
    SpokeConnectInboundSequence **out_handle,
    SpokeConnectError *out_error);

/**
 * Advances the expectation over `sequence` and writes the next expected value
 * to `out_next_expected`. A mismatch reports
 * `SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH`.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_inbound_sequence_advance(
    const SpokeConnectInboundSequence *handle,
    int64_t sequence,
    uint64_t *out_next_expected,
    SpokeConnectError *out_error);

/** Releases an inbound sequence expectation. A NULL handle is a no-op. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_inbound_sequence_free(
    SpokeConnectInboundSequence *handle);

/* ── Foreign callback transports and loopback hosts ────────────────────── */

/**
 * Creates a transport handle over a host callback table. The table is copied;
 * on success ownership of `user_data` transfers and its `destroy` runs
 * exactly once after the last reference and in-flight callback is gone. On
 * failure there is no transfer and no destroy call.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_transport_new(
    const SpokeConnectTransportTable *table,
    void *user_data,
    SpokeConnectTransport **out_transport,
    SpokeConnectError *out_error);

/** Releases a foreign transport handle. A NULL handle is a no-op. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_transport_free(
    SpokeConnectTransport *transport);

/** Creates a back-to-back in-memory loopback transport pair. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_loopback_transport_pair_new(
    SpokeConnectLoopbackTransportPair **out_pair,
    SpokeConnectError *out_error);

/**
 * Borrows the client end of a loopback pair into a new owned handle. The pair
 * keeps its own reference.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_loopback_transport_pair_client(
    const SpokeConnectLoopbackTransportPair *pair,
    SpokeConnectLoopbackTransport **out_transport,
    SpokeConnectError *out_error);

/** Borrows the server end of a loopback pair into a new owned handle. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_loopback_transport_pair_server(
    const SpokeConnectLoopbackTransportPair *pair,
    SpokeConnectLoopbackTransport **out_transport,
    SpokeConnectError *out_error);

/** Releases a loopback pair. A NULL handle is a no-op. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_loopback_transport_pair_free(
    SpokeConnectLoopbackTransportPair *pair);

/** Sends one envelope to the peer end of the loopback connection. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_loopback_transport_send(
    const SpokeConnectLoopbackTransport *transport,
    SpokeConnectSlice envelope,
    SpokeConnectError *out_error);

/**
 * Receives the next envelope from the loopback connection into
 * `out_envelope`; blocks until one arrives or the connection closes.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_loopback_transport_recv(
    const SpokeConnectLoopbackTransport *transport,
    SpokeConnectBuffer *out_envelope,
    SpokeConnectError *out_error);

/** Closes the whole loopback connection (both directions). Idempotent. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_loopback_transport_close(
    const SpokeConnectLoopbackTransport *transport,
    SpokeConnectError *out_error);

/** Releases a loopback transport end. A NULL handle is a no-op. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_loopback_transport_free(
    SpokeConnectLoopbackTransport *transport);

/* ── Remote adapter ────────────────────────────────────────────────────── */

/**
 * Dials a remote peer over `transport` and returns the established adapter.
 *
 * `transport` is borrowed and cloned internally. `local_seed` is the 32-byte
 * Ed25519 identity seed; `local_manifest_json` the canonical JSON of the
 * local host capability manifest; `remote_pubkey` the 32-byte Ed25519 key of
 * the peer; `allowlist` the accepted peer ids. `invoke_timeout_ms` follows
 * the optional-scalar rule.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_new(
    const SpokeConnectTransport *transport,
    SpokeConnectSlice local_seed,
    SpokeConnectSlice local_manifest_json,
    SpokeConnectSlice remote_pubkey,
    const SpokeConnectSlice *allowlist,
    size_t allowlist_count,
    SpokeConnectOptionalU64 invoke_timeout_ms,
    SpokeConnectRemoteAdapter **out_adapter,
    SpokeConnectError *out_error);

/** Releases an adapter handle. A NULL handle is a no-op; this is not close. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_remote_adapter_free(
    SpokeConnectRemoteAdapter *adapter);

/** One of `Disconnected` / `Handshaking` / `Established` / `Closed`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_state(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectBuffer *out_state,
    SpokeConnectError *out_error);

/** The session id, absent until the session is established. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_session_id(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectOptionalBuffer *out_session_id,
    SpokeConnectError *out_error);

/** The remote peer id, absent until the session is established. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_remote_peer_id(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectOptionalBuffer *out_peer_id,
    SpokeConnectError *out_error);

/** The remote host capability manifest JSON, absent before establish. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_remote_manifest(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectOptionalBuffer *out_manifest_json,
    SpokeConnectError *out_error);

/** The local host capability manifest as JSON. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_get_host_capability_manifest(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `getKnowledgeEntry`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_get_knowledge_entry(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice entry_id,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `putKnowledgeEntry` with an optional base revision. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_put_knowledge_entry(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice entry_json,
    SpokeConnectOptionalU64 expected_base_revision,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `getRelation`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_get_relation(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice relation_id,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `putRelation` with an optional base revision. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_put_relation(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice relation_json,
    SpokeConnectOptionalU64 expected_base_revision,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `listKnowledgeEntries`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_list_knowledge_entries(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice scope_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `listTimelineEvents`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_list_timeline_events(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice scope_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `putFindings`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_put_findings(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice findings_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `listRules` over an array of rule references. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_list_rules(
    const SpokeConnectRemoteAdapter *adapter,
    const SpokeConnectSlice *rule_refs,
    size_t rule_refs_count,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Baseline ports: `listPeerHostCapabilityManifests`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_list_peer_host_capability_manifests(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Optional ports: `port.computable.project`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_project(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice project_request_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Optional ports: `port.computable.compute`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_compute(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice compute_request_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Optional ports: `port.fork.listTimelineEvents`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_list_fork_timeline_events(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice scope_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Core-op extract service face. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_extract(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice extract_request_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Invokes a capability on the remote peer by capability id. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_invoke_tool(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice capability_id,
    SpokeConnectSlice arguments_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/**
 * Registers a dialer-side tool handler for `capability_id`, so the peer can
 * invoke it in reverse. The handler is borrowed and cloned internally.
 * A `capability_id` outside the tool grammar rejects without side effect.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_register_tool_handler(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectSlice capability_id,
    const SpokeConnectToolHandler *handler,
    SpokeConnectError *out_error);

/**
 * Closes the adapter's session and releases its resources. Distinct from
 * `spoke_connect_remote_adapter_free`; idempotent.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_remote_adapter_close(
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectError *out_error);

/* ── Multi-peer router ─────────────────────────────────────────────────── */

/** Creates an empty multi-peer router. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_new(
    SpokeConnectMultiPeerRouter **out_router,
    SpokeConnectError *out_error);

/**
 * Releases a router handle. A NULL handle is a no-op. This releases the
 * router's references; it does not close caller-owned adapters.
 */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_free(
    SpokeConnectMultiPeerRouter *router);

/**
 * Registers an adapter, borrowing it and retaining a reference. The peer id
 * is written to `out_peer_id` as a UTF-8 owned buffer.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_register_peer(
    const SpokeConnectMultiPeerRouter *router,
    const SpokeConnectRemoteAdapter *adapter,
    SpokeConnectBuffer *out_peer_id,
    SpokeConnectError *out_error);

/** Unregisters a peer id; the adapter itself stays caller-owned. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_unregister_peer(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice peer_id,
    SpokeConnectError *out_error);

/** The registered peer ids as a JSON array of strings. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_list_peers(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** The composed host capability manifest as JSON. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_get_host_capability_manifest(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `getKnowledgeEntry`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_get_knowledge_entry(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice entry_id,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `putKnowledgeEntry` with an optional base revision. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_put_knowledge_entry(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice entry_json,
    SpokeConnectOptionalU64 expected_base_revision,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `getRelation`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_get_relation(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice relation_id,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `putRelation` with an optional base revision. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_put_relation(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice relation_json,
    SpokeConnectOptionalU64 expected_base_revision,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `listKnowledgeEntries`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_list_knowledge_entries(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice scope_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `listTimelineEvents`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_list_timeline_events(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice scope_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `putFindings`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_put_findings(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice findings_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `listRules` over an array of rule references. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_list_rules(
    const SpokeConnectMultiPeerRouter *router,
    const SpokeConnectSlice *rule_refs,
    size_t rule_refs_count,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Routed baseline ports: `listPeerHostCapabilityManifests`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_list_peer_host_capability_manifests(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/**
 * Routes a tool invoke to a capable peer. Optional port families
 * (`port.computable.*` / `port.fork.*`) ride the per-peer adapter instead.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_multi_peer_router_invoke_tool(
    const SpokeConnectMultiPeerRouter *router,
    SpokeConnectSlice capability_id,
    SpokeConnectSlice arguments_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/* ── Serving side: ports, tools and responder ──────────────────────────── */

/**
 * Creates a ports-handler handle over a host callback table. Every table
 * pointer is required. On success ownership of `user_data` transfers and its
 * `destroy` runs exactly once after the last reference and in-flight callback
 * is gone; on failure there is no transfer and no destroy call.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_ports_handler_new(
    const SpokeConnectPortsHandlerTable *table,
    void *user_data,
    SpokeConnectPortsHandler **out_handler,
    SpokeConnectError *out_error);

/** Releases a ports handler. A NULL handle is a no-op. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_ports_handler_free(
    SpokeConnectPortsHandler *handler);

/** Creates a tool-handler handle over a host callback table. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_tool_handler_new(
    const SpokeConnectToolHandlerTable *table,
    void *user_data,
    SpokeConnectToolHandler **out_handler,
    SpokeConnectError *out_error);

/** Releases a tool handler. A NULL handle is a no-op. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_tool_handler_free(
    SpokeConnectToolHandler *handler);

/**
 * Serves an accepted connection on `transport` as the responder.
 *
 * `transport` is borrowed and cloned internally. `local_seed` is the 32-byte
 * Ed25519 identity seed; `local_manifest_json` the canonical JSON of the
 * local host capability manifest; `allowlist` the accepted peer ids;
 * `peer_keys` the peer id to 32-byte public key table (duplicate peer ids
 * reject as invalid input). `ports` is an optional ports handler — NULL means
 * the responder serves no ports callbacks. `invoke_timeout_ms` follows the
 * optional-scalar rule.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_responder_new(
    const SpokeConnectTransport *transport,
    SpokeConnectSlice local_seed,
    SpokeConnectSlice local_manifest_json,
    const SpokeConnectSlice *allowlist,
    size_t allowlist_count,
    const SpokeConnectPeerKey *peer_keys,
    size_t peer_key_count,
    const SpokeConnectPortsHandler *ports,
    SpokeConnectOptionalU64 invoke_timeout_ms,
    SpokeConnectResponder **out_responder,
    SpokeConnectError *out_error);

/** Releases a responder handle. A NULL handle is a no-op; this is not close. */
SPOKE_CONNECT_API void SPOKE_CONNECT_CALL spoke_connect_responder_free(
    SpokeConnectResponder *responder);

/** One of `Disconnected` / `Handshaking` / `Established` / `Closed`. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_responder_state(
    const SpokeConnectResponder *responder,
    SpokeConnectBuffer *out_state,
    SpokeConnectError *out_error);

/** The session id, absent until the session is established. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_responder_session_id(
    const SpokeConnectResponder *responder,
    SpokeConnectOptionalBuffer *out_session_id,
    SpokeConnectError *out_error);

/** The dialer's peer id, absent until the session is established. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_responder_remote_peer_id(
    const SpokeConnectResponder *responder,
    SpokeConnectOptionalBuffer *out_peer_id,
    SpokeConnectError *out_error);

/** The dialer's host capability manifest JSON, absent before establish. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_responder_remote_manifest(
    const SpokeConnectResponder *responder,
    SpokeConnectOptionalBuffer *out_manifest_json,
    SpokeConnectError *out_error);

/**
 * Registers a served tool for `capability_id`; registration is last-wins.
 * The handler is borrowed and cloned internally. A `capability_id` outside
 * the tool grammar rejects without side effect.
 */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_responder_register_tool_handler(
    const SpokeConnectResponder *responder,
    SpokeConnectSlice capability_id,
    const SpokeConnectToolHandler *handler,
    SpokeConnectError *out_error);

/** Invokes the peer's tool for `capability_id` from the serving side. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_responder_invoke_tool(
    const SpokeConnectResponder *responder,
    SpokeConnectSlice capability_id,
    SpokeConnectSlice arguments_json,
    SpokeConnectBuffer *out_json,
    SpokeConnectError *out_error);

/** Closes the responder's session. Distinct from free; idempotent. */
SPOKE_CONNECT_API int32_t SPOKE_CONNECT_CALL spoke_connect_responder_close(
    const SpokeConnectResponder *responder,
    SpokeConnectError *out_error);
/* SPOKE_CONNECT_DECLARATIONS_END */

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* SPOKE_CONNECT_H */
