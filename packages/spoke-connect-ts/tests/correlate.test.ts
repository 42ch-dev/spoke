import { describe, expect, it } from "vitest";

import type {
  ConnectInvokeRequest,
  ConnectInvokeResponse,
} from "@42ch/spoke-schemas";

import {
  checkResponseCorrelation,
  correlationFromRequest,
  correlationFromResponse,
} from "../src/core/correlate.js";
import {
  authenticateInvokeRequest,
  authenticateInvokeResponse,
} from "../src/core/envelope-auth.js";

/**
 * Deterministic 32-byte test identity seed (mirrors the Rust
 * `correlate.rs` test module convention — no key material as a literal in a
 * crypto call site).
 */
const TEST_SEED: Uint8Array = Uint8Array.from({ length: 32 }, () => 0x2b);

async function request(
  sequence: number,
  requestId: string,
  sessionId: string,
): Promise<ConnectInvokeRequest> {
  return authenticateInvokeRequest(TEST_SEED, {
    session_id: sessionId,
    sequence,
    request_id: requestId,
    op: "check",
    payload: { findings: [] },
  });
}

async function successResponse(
  sequence: number,
  requestId: string,
  sessionId: string,
): Promise<ConnectInvokeResponse> {
  return authenticateInvokeResponse(TEST_SEED, {
    session_id: sessionId,
    sequence,
    request_id: requestId,
    payload: { findings: [] },
  });
}

async function errorResponse(
  sequence: number,
  requestId: string,
  sessionId: string,
): Promise<ConnectInvokeResponse> {
  return authenticateInvokeResponse(TEST_SEED, {
    session_id: sessionId,
    sequence,
    request_id: requestId,
    error: {
      code: "check_failed",
      message: "spike check failed",
      extensions: {},
    },
  });
}

describe("correlation (port of correlate.rs)", () => {
  it("exposes the request echo fields", async () => {
    expect(
      correlationFromRequest(await request(0, "req-1", "sess-1")),
    ).toEqual({
      session_id: "sess-1",
      sequence: 0,
      request_id: "req-1",
    });
  });

  it("exposes the echo fields on both response branches", async () => {
    expect(
      correlationFromResponse(await successResponse(0, "req-1", "sess-1")),
    ).toEqual({
      session_id: "sess-1",
      sequence: 0,
      request_id: "req-1",
    });
    expect(
      correlationFromResponse(await errorResponse(3, "req-9", "sess-2")),
    ).toEqual({
      session_id: "sess-2",
      sequence: 3,
      request_id: "req-9",
    });
  });

  it("accepts an exact echo", async () => {
    const expected = correlationFromRequest(await request(0, "req-1", "sess-1"));
    const echoed = correlationFromResponse(
      await successResponse(0, "req-1", "sess-1"),
    );
    expect(() => checkResponseCorrelation(expected, echoed)).not.toThrow();
  });

  it("rejects a sequence mismatch", async () => {
    const expected = correlationFromRequest(await request(0, "req-1", "sess-1"));
    const echoed = correlationFromResponse(
      await successResponse(1, "req-1", "sess-1"),
    );
    expect(() =>
      checkResponseCorrelation(expected, echoed),
    ).toThrowError(expect.objectContaining({ code: "correlation_mismatch" }));
  });

  it("rejects a request_id mismatch", async () => {
    const expected = correlationFromRequest(await request(0, "req-1", "sess-1"));
    const echoed = correlationFromResponse(
      await successResponse(0, "other-req", "sess-1"),
    );
    expect(() =>
      checkResponseCorrelation(expected, echoed),
    ).toThrowError(expect.objectContaining({ code: "correlation_mismatch" }));
  });

  it("rejects a session_id mismatch on the error branch too", async () => {
    const expected = correlationFromRequest(await request(0, "req-1", "sess-1"));
    const echoed = correlationFromResponse(
      await errorResponse(0, "req-1", "other-session"),
    );
    expect(() =>
      checkResponseCorrelation(expected, echoed),
    ).toThrowError(expect.objectContaining({ code: "correlation_mismatch" }));
  });
});
