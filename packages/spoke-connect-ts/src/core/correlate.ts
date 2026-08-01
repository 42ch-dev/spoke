/**
 * Response correlation: echo checks for `session_id`, `sequence`, and
 * `request_id`.
 *
 * Ported from `crates/spoke-connect/src/core/correlate.rs`; normative rule
 * `.mstar/specs/spoke-connect.md` §Ordering semantics: a response MUST echo
 * `session_id`, `sequence`, and `request_id` from the request; any mismatch
 * is a correlation failure (local error).
 */

import type { ConnectInvokeRequest, ConnectInvokeResponse } from "@42ch/spoke-schemas";

import { CoreInvokeError } from "./error.js";

/** The minimal echo material needed to correlate a response with its request. */
export interface Correlation {
  session_id: string;
  sequence: number;
  request_id: string;
}

/** The request's wire echo fields. */
export function correlationFromRequest(request: ConnectInvokeRequest): Correlation {
  return {
    session_id: request.session_id,
    sequence: request.sequence,
    request_id: request.request_id,
  };
}

/** The response's wire echo fields (both success and error branches). */
export function correlationFromResponse(response: ConnectInvokeResponse): Correlation {
  return {
    session_id: response.session_id,
    sequence: response.sequence,
    request_id: response.request_id,
  };
}

/**
 * Check that `actual` (a response's echo fields) matches `expected` (the
 * request's echo fields) on all three fields; throws
 * `correlation_mismatch` otherwise.
 */
export function checkResponseCorrelation(expected: Correlation, actual: Correlation): void {
  if (
    expected.session_id !== actual.session_id ||
    expected.sequence !== actual.sequence ||
    expected.request_id !== actual.request_id
  ) {
    throw new CoreInvokeError("correlation_mismatch");
  }
}
