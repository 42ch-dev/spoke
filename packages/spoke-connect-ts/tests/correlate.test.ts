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

function request(sequence: number, requestId: string, sessionId: string): ConnectInvokeRequest {
  return {
    session_id: sessionId,
    sequence,
    request_id: requestId,
    op: "check",
    payload: { findings: [] },
    extensions: {},
  };
}

function successResponse(
  sequence: number,
  requestId: string,
  sessionId: string,
): ConnectInvokeResponse {
  return {
    session_id: sessionId,
    sequence,
    request_id: requestId,
    payload: { findings: [] },
    extensions: {},
  };
}

function errorResponse(
  sequence: number,
  requestId: string,
  sessionId: string,
): ConnectInvokeResponse {
  return {
    session_id: sessionId,
    sequence,
    request_id: requestId,
    error: {
      code: "check_failed",
      message: "spike check failed",
      extensions: {},
    },
    extensions: {},
  };
}

describe("correlation (port of correlate.rs)", () => {
  it("exposes the request echo fields", () => {
    expect(correlationFromRequest(request(0, "req-1", "sess-1"))).toEqual({
      session_id: "sess-1",
      sequence: 0,
      request_id: "req-1",
    });
  });

  it("exposes the echo fields on both response branches", () => {
    expect(correlationFromResponse(successResponse(0, "req-1", "sess-1"))).toEqual({
      session_id: "sess-1",
      sequence: 0,
      request_id: "req-1",
    });
    expect(correlationFromResponse(errorResponse(3, "req-9", "sess-2"))).toEqual({
      session_id: "sess-2",
      sequence: 3,
      request_id: "req-9",
    });
  });

  it("accepts an exact echo", () => {
    const expected = correlationFromRequest(request(0, "req-1", "sess-1"));
    expect(() =>
      checkResponseCorrelation(
        expected,
        correlationFromResponse(successResponse(0, "req-1", "sess-1")),
      ),
    ).not.toThrow();
  });

  it("rejects a sequence mismatch", () => {
    const expected = correlationFromRequest(request(0, "req-1", "sess-1"));
    expect(() =>
      checkResponseCorrelation(
        expected,
        correlationFromResponse(successResponse(1, "req-1", "sess-1")),
      ),
    ).toThrowError(expect.objectContaining({ code: "correlation_mismatch" }));
  });

  it("rejects a request_id mismatch", () => {
    const expected = correlationFromRequest(request(0, "req-1", "sess-1"));
    expect(() =>
      checkResponseCorrelation(
        expected,
        correlationFromResponse(successResponse(0, "other-req", "sess-1")),
      ),
    ).toThrowError(expect.objectContaining({ code: "correlation_mismatch" }));
  });

  it("rejects a session_id mismatch on the error branch too", () => {
    const expected = correlationFromRequest(request(0, "req-1", "sess-1"));
    expect(() =>
      checkResponseCorrelation(
        expected,
        correlationFromResponse(errorResponse(0, "req-1", "other-session")),
      ),
    ).toThrowError(expect.objectContaining({ code: "correlation_mismatch" }));
  });
});
