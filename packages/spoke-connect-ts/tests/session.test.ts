import { describe, expect, it } from "vitest";

import { negotiatedCapabilities, Session } from "../src/core/session.js";

function baselineSession(): Session {
  return new Session({
    session_id: "sess-1",
    initiator_peer_id: "peer-b",
    responder_peer_id: "peer-a",
    negotiated_capabilities: ["spoke-baseline"],
  });
}

describe("Session helper (AD-P0-3 thin state)", () => {
  it("starts with outbound 0 and inbound expected 0", () => {
    const session = baselineSession();
    expect(session.nextOutboundSequence).toBe(0);
    expect(session.nextExpectedInboundSequence).toBe(0);
    expect(session.session_id).toBe("sess-1");
    expect(session.initiator_peer_id).toBe("peer-b");
    expect(session.responder_peer_id).toBe("peer-a");
  });

  it("allocates outbound sequences starting at 0", () => {
    const session = baselineSession();
    expect(session.allocateOutboundSequence()).toBe(0);
    expect(session.allocateOutboundSequence()).toBe(1);
    expect(session.allocateOutboundSequence()).toBe(2);
    expect(session.nextOutboundSequence).toBe(3);
  });

  it("accepts inbound sequences starting at 0", () => {
    const session = baselineSession();
    session.acceptInboundSequence(0);
    session.acceptInboundSequence(1);
    session.acceptInboundSequence(2);
    expect(session.nextExpectedInboundSequence).toBe(3);
  });

  it("rejects an inbound replay with inbound_sequence_mismatch", () => {
    const session = baselineSession();
    session.acceptInboundSequence(0);
    expect(() => session.acceptInboundSequence(0)).toThrowError(
      expect.objectContaining({
        code: "inbound_sequence_mismatch",
        details: { expected: 1, actual: 0 },
      }),
    );
  });

  it("evaluates the dispatch gate against the session's negotiated capabilities", () => {
    const session = baselineSession();
    expect(session.dispatchAllowed("check")).toBe(true);
    expect(session.dispatchAllowed("compute")).toBe(false); // l2-computable not negotiated
    expect(session.dispatchAllowed("custom-op")).toBe(false); // unknown op fail-closed
  });
});

describe("negotiatedCapabilities (agreed subset)", () => {
  it("intersects both hosts' capability lists in local order", () => {
    expect(
      negotiatedCapabilities(["spoke-baseline", "l2-computable"], ["spoke-baseline"]),
    ).toEqual(["spoke-baseline"]);
    expect(
      negotiatedCapabilities(
        ["spoke-connect", "spoke-baseline"],
        ["spoke-baseline", "spoke-connect"],
      ),
    ).toEqual(["spoke-connect", "spoke-baseline"]);
  });

  it("is empty when the sets are disjoint", () => {
    expect(negotiatedCapabilities(["l2-computable"], ["spoke-baseline"])).toEqual([]);
    expect(negotiatedCapabilities([], ["spoke-baseline"])).toEqual([]);
  });
});
