/**
 * Pure session-core error types (local, not wire).
 *
 * Mirrors `crates/spoke-connect/src/core/error.rs` variants as string-code
 * discriminators on two error classes:
 * - `CoreError` — hello-gate / identity failures (hello verify, nonce,
 *   handshake, crypto, JCS).
 * - `CoreInvokeError` — invoke path (sequence, correlation).
 *
 * The `code` field is a string-code union naming the Rust variant
 * (`invalid_hello_signature`, `nonce_replay`, `sequence_exhausted`,
 * `inbound_sequence_mismatch`, `correlation_mismatch`, …). Callers
 * discriminate on `code`; wire `ErrorEnvelope.code` mapping happens only on
 * the server invoke path (`op_unsupported` for dispatch deny).
 */

/** String codes mirroring `CoreError` variants. */
export type CoreErrorCode =
  | "invalid_hello_signature"
  | "nonce_replay"
  | "handshake_failed"
  | "invalid_nonce"
  | "crypto"
  | "jcs";

/** String codes mirroring `CoreInvokeError` variants. */
export type CoreInvokeErrorCode =
  | "sequence_exhausted"
  | "inbound_sequence_mismatch"
  | "correlation_mismatch";

const CORE_ERROR_MESSAGES: Record<CoreErrorCode, string> = {
  invalid_hello_signature: "hello signature invalid",
  nonce_replay: "hello nonce replayed",
  handshake_failed: "handshake failed",
  invalid_nonce: "invalid hello nonce",
  crypto: "crypto error",
  jcs: "JCS canonicalization failed",
};

const CORE_INVOKE_ERROR_MESSAGES: Record<CoreInvokeErrorCode, string> = {
  sequence_exhausted: "sequence space exhausted — reopen session",
  inbound_sequence_mismatch: "inbound sequence mismatch",
  correlation_mismatch: "request/response mismatch",
};

/** Hello-gate / identity failure (mirrors `CoreError`). */
export class CoreError extends Error {
  readonly code: CoreErrorCode;
  /** Optional structured context (e.g. handshake reason strings). */
  readonly details?: Record<string, unknown>;

  constructor(code: CoreErrorCode, message?: string, details?: Record<string, unknown>) {
    super(message ?? CORE_ERROR_MESSAGES[code]);
    this.name = "CoreError";
    this.code = code;
    this.details = details;
  }
}

/** Invoke-path failure over an established session (mirrors `CoreInvokeError`). */
export class CoreInvokeError extends Error {
  readonly code: CoreInvokeErrorCode;
  /** Optional structured context (e.g. `{ expected, actual }` for sequence mismatch). */
  readonly details?: Record<string, unknown>;

  constructor(code: CoreInvokeErrorCode, message?: string, details?: Record<string, unknown>) {
    super(message ?? CORE_INVOKE_ERROR_MESSAGES[code]);
    this.name = "CoreInvokeError";
    this.code = code;
    this.details = details;
  }
}
