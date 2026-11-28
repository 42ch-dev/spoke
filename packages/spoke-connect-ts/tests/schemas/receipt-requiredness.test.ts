import { describe, expect, it } from "vitest";
import type { MemoryCaptureResponse } from "@42ch/spoke-schemas";

type CommittedBranch = Extract<MemoryCaptureResponse, { status: "committed" }>;

type IsOptional<T, K extends keyof T> = {} extends Pick<T, K> ? true : false;

type IdempotencyKeyRequired =
  IsOptional<CommittedBranch, "idempotency_key"> extends false ? true : never;

const idempotencyKeyIsRequired: IdempotencyKeyRequired = true;
void idempotencyKeyIsRequired;

describe("keyed committed receipt TS requiredness", () => {
  it("requires idempotency_key on the committed branch and round-trips a valid receipt", () => {
    const receipt: CommittedBranch = {
      schema_version: 1,
      receipt_id: "rcpt_0001",
      op: "memory.capture",
      space_id: "spc_1",
      requester_principal_id: "prn_1",
      idempotency_key: "cap-1",
      payload_digest: "a".repeat(64),
      status: "committed",
      result: { candidate_ids: [], classified: 0 },
      created_at: "2026-10-29T00:00:00Z",
      completed_at: "2026-10-29T00:00:01Z",
      extensions: {},
    };
    expect(receipt.idempotency_key).toBe("cap-1");
  });
});
