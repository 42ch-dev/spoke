import type { ErrorEnvelope } from "@42ch/spoke-schemas";

import { spokeReject, type SpokeReject, SpokeRejectCode } from "../result.js";

const SPOKE_REJECT_CODE_VALUES = new Set<string>(Object.values(SpokeRejectCode));

function isSpokeRejectCode(code: string): code is SpokeRejectCode {
  return SPOKE_REJECT_CODE_VALUES.has(code);
}

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
  if (!isSpokeRejectCode(error.code)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `Unknown error code: ${error.code}`,
      { wire_code: error.code },
    );
  }

  return {
    ok: false,
    code: error.code,
    message: error.message,
    ...(error.details !== undefined ? { details: error.details } : {}),
  };
}
