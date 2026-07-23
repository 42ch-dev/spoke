import { describe, expect, it } from "vitest";

import { spokeReject, SpokeRejectCode } from "../result.js";
import { fromErrorEnvelope, toErrorEnvelope } from "./envelope.js";

const V0_ITER004_CODES = [
  SpokeRejectCode.INVALID_INPUT,
  SpokeRejectCode.MISSING_REQUIRED_FIELD,
  SpokeRejectCode.REVISION_CONFLICT,
  SpokeRejectCode.STORED_REVISION_STALE,
  SpokeRejectCode.KEYBLOCK_NOT_FOUND,
  SpokeRejectCode.KEYBLOCK_ALREADY_EXISTS,
  SpokeRejectCode.KEYBLOCK_TERMINAL_STATUS,
  SpokeRejectCode.RELATION_SELF_EDGE,
  SpokeRejectCode.RELATION_MISSING_ENDPOINT,
  SpokeRejectCode.DUPLICATE_ACTIVE_KEYBLOCK,
] as const;

const V0_ITER002_CODES = [
  SpokeRejectCode.INVALID_STATUS,
  SpokeRejectCode.INVALID_STATUS_TRANSITION,
  SpokeRejectCode.CANDIDATE_NOT_PROVISIONAL,
  SpokeRejectCode.CANDIDATE_TERMINAL_STATUS,
  SpokeRejectCode.EMPTY_CANONICAL_NAME,
  SpokeRejectCode.MERGE_TARGET_SELF,
  SpokeRejectCode.INVALID_PACKET_INPUT,
  SpokeRejectCode.INVALID_KEYBLOCK_STATUS,
  SpokeRejectCode.INVALID_KEYBLOCK_STATUS_TRANSITION,
] as const;

describe("toErrorEnvelope / fromErrorEnvelope", () => {
  it.each([...V0_ITER002_CODES, ...V0_ITER004_CODES])(
    "round-trips code %s",
    (code) => {
      const reject = spokeReject(code, `message for ${code}`, { sample: true });
      const envelope = toErrorEnvelope(reject);

      expect(envelope.extensions).toEqual({});
      expect(envelope.code).toBe(code);
      expect(envelope.message).toBe(`message for ${code}`);
      expect(envelope.details).toEqual({ sample: true });

      const roundTrip = fromErrorEnvelope(envelope);
      expect(roundTrip).toEqual(reject);
    },
  );

  it("omits details when absent on outbound map", () => {
    const reject = spokeReject(SpokeRejectCode.INVALID_INPUT, "no details");
    const envelope = toErrorEnvelope(reject);

    expect(envelope.details).toBeUndefined();
    expect(fromErrorEnvelope(envelope)).toEqual(reject);
  });
});
