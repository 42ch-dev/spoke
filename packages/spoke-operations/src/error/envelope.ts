import type { ErrorEnvelope } from "@42ch/spoke-schemas";

import type { SpokeReject, SpokeRejectCode } from "../result.js";

/**
 * Map SpokeReject to ops ErrorEnvelope wire shape.
 */
export function toErrorEnvelope(reject: SpokeReject): ErrorEnvelope {
  return {
    code: reject.code,
    message: reject.message,
    ...(reject.details !== undefined ? { details: reject.details } : {}),
    extensions: {},
  };
}

/**
 * Map ErrorEnvelope back to SpokeReject.
 */
export function fromErrorEnvelope(error: ErrorEnvelope): SpokeReject {
  return {
    ok: false,
    code: error.code as SpokeRejectCode,
    message: error.message,
    ...(error.details !== undefined ? { details: error.details } : {}),
  };
}
