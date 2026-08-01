import { describe, expect, it } from "vitest";

import {
  decodeJsonMessage,
  encodeJsonMessage,
} from "../src/framing.js";

describe("connect framing (one JSON document per WebSocket message)", () => {
  it("round-trips a JSON document through encode/decode", () => {
    const doc = {
      protocol_version: 1,
      peer_id: "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf",
      nonce: "golden-nonce-000000000001",
    };
    const frame = encodeJsonMessage(doc);
    expect(typeof frame).toBe("string");
    expect(decodeJsonMessage(frame)).toEqual(doc);
  });

  it("decodes binary frames (UTF-8 JSON bytes) as one document", () => {
    const doc = { ok: true, sequence: 0 };
    const bytes = new TextEncoder().encode(encodeJsonMessage(doc));
    expect(decodeJsonMessage(bytes)).toEqual(doc);
    expect(decodeJsonMessage(bytes.buffer)).toEqual(doc);
  });

  it("enforces one document per frame (trailing bytes fail closed)", () => {
    const frame = encodeJsonMessage({ a: 1 }) + '{"b":2}';
    expect(() => decodeJsonMessage(frame)).toThrow(SyntaxError);
  });

  it("rejects non-serializable documents on encode", () => {
    expect(() => encodeJsonMessage(undefined)).toThrow(
      /not JSON-serializable/,
    );
  });
});
