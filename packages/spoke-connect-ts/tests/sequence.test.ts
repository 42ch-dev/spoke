import { describe, expect, it } from "vitest";

import {
  InboundSequence,
  MAX_SEQUENCE,
  OutboundSequence,
} from "../src/core/sequence.js";
import { CoreInvokeError } from "../src/core/error.js";

describe("OutboundSequence (port of sequence.rs)", () => {
  it("starts at zero and increments monotonically", () => {
    const seq = new OutboundSequence();
    expect(seq.allocate()).toBe(0);
    expect(seq.allocate()).toBe(1);
    expect(seq.allocate()).toBe(2);
    expect(seq.next()).toBe(3);
  });

  it("throws sequence_exhausted past 2^53-1 without wrapping", () => {
    const seq = new OutboundSequence();
    seq.setNext(MAX_SEQUENCE);
    expect(seq.allocate()).toBe(MAX_SEQUENCE); // last valid allocation
    expect(() => seq.allocate()).toThrowError(
      expect.objectContaining({ code: "sequence_exhausted" }),
    );
    // Still exhausted — no wrap-around.
    expect(() => seq.allocate()).toThrowError(
      expect.objectContaining({ code: "sequence_exhausted" }),
    );
  });

  it("MAX_SEQUENCE is the JSON-safe wire maximum 2^53-1", () => {
    expect(MAX_SEQUENCE).toBe(2 ** 53 - 1);
    expect(MAX_SEQUENCE).toBe(9007199254740991);
  });
});

describe("InboundSequence (port of sequence.rs)", () => {
  it("accepts sequential sequences", () => {
    const inbound = new InboundSequence();
    expect(inbound.advance(0)).toBe(1);
    expect(inbound.advance(1)).toBe(2);
    expect(inbound.advance(2)).toBe(3);
    expect(inbound.nextExpected()).toBe(3);
  });

  it("rejects replay and out-of-order with expected/actual details", () => {
    const inbound = new InboundSequence();
    expect(inbound.advance(0)).toBe(1);

    // Replay of an already-consumed sequence.
    expect(() => inbound.advance(0)).toThrowError(
      expect.objectContaining({
        code: "inbound_sequence_mismatch",
        details: { expected: 1, actual: 0 },
      }),
    );

    // Out-of-order (skipping ahead).
    expect(() => inbound.advance(2)).toThrowError(
      expect.objectContaining({
        code: "inbound_sequence_mismatch",
        details: { expected: 1, actual: 2 },
      }),
    );

    // The expectation is unchanged after rejections.
    expect(inbound.nextExpected()).toBe(1);
  });

  it("rejects negative sequences", () => {
    const inbound = new InboundSequence();
    expect(() => inbound.advance(-1)).toThrowError(
      expect.objectContaining({
        code: "inbound_sequence_mismatch",
        details: { expected: 0, actual: -1 },
      }),
    );
  });

  it("rejects sequences above the wire maximum", () => {
    const inbound = new InboundSequence();
    expect(() => inbound.advance(MAX_SEQUENCE + 1)).toThrowError(CoreInvokeError);
  });
});
