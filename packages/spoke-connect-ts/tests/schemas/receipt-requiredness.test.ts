/**
 * Reviewer-round regression: the per-op committed receipt branch must keep
 * `idempotency_key` REQUIRED in the TypeScript output.
 *
 * Mechanism: COMPILE-LEVEL. The schema pins the key through a flattened
 * branch definition (schemas/ops/memory-*-response.schema.json); when the
 * generator drops the required-ness (the old allOf-of-fragments shape), the
 * `IdempotencyKeyRequired` assertion below stops compiling and
 * `tsc --noEmit` fails. Vitest additionally runs the positive runtime check
 * that a well-formed committed receipt still type-checks and round-trips.
 */
import { describe, expect, it } from "vitest";
import type { MemoryCaptureResponse } from "@42ch/spoke-schemas";

type CommittedBranch = Extract<MemoryCaptureResponse, { status: "committed" }>;

/** `true` when `K` is optional (or undefined-accepting) on `T`. */
type IsOptional<T, K extends keyof T> = {} extends Pick<T, K> ? true : false;

/** `true` only while `idempotency_key` stays required on the committed branch. */
type IdempotencyKeyRequired =
  IsOptional<CommittedBranch, "idempotency_key"> extends false ? true : never;

// Compile-level assertion — tsc fails here if the generator regresses.
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
