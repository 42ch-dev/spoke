import { describe, expect, it } from "vitest";
import canonicalize from "canonicalize";

import { jcsSha256Hex, jcsSha256HexExcluding } from "./digests.js";
import type { SpokeReject, SpokeResult } from "../result.js";

function unwrap(result: SpokeResult<string>): string {
  if (!result.ok) {
    throw new Error(`expected ok, got ${JSON.stringify(result)}`);
  }
  return result.value;
}

function unwrapReject(result: SpokeResult<string>): SpokeReject {
  if (result.ok) {
    throw new Error("expected reject");
  }
  return result;
}

describe("memory digests (RFC 8785 JCS + SHA-256)", () => {
  it("matches the Rust pinned known vectors", () => {
    expect(
      unwrap(jcsSha256Hex({ b: 1, a: [true, null] })),
    ).toBe("51705a2c9eb3e7e410a58f696a770c3ac3885a0cf43eb7fc88f5e47c11d4d30d");

    expect(
      unwrap(
        jcsSha256Hex({
          numbers: { int: 42, float: 1.5 },
          literals: [null, true, false],
        }),
      ),
    ).toBe("fcd9991ad91f6c352701dd3cd819b0633b16e65757773973e7683318c11626f0");
  });

  it("spells exponent-literal floats without exponent like Rust serde_jcs", () => {
    expect(canonicalize(1e16)).toBe("10000000000000000");
    expect(canonicalize(1e20)).toBe("100000000000000000000");
    expect(unwrap(jcsSha256Hex(1e16))).toBe(
      "139eb393675707818651f879828a526159209ca3ad3b2f94f9f8ec8c4fb5e610",
    );
    expect(unwrap(jcsSha256Hex(1e20))).toBe(
      "c344e9487bfbd5c4e03c9fb90d62a5dde5e00b54d55c46e9f4a803aea162b80c",
    );
    expect(
      unwrap(jcsSha256Hex({ product: { magnitude: 1e20 } })),
    ).toBe(
      "7aa1fea2dbfc1627cc71a0bde99d769bda6f99a5d7dd06e7808743aae6e16ace",
    );
  });

  it("sorts non-BMP member names by UTF-16 code units like Rust", () => {
    const value = { "\u{1F600}": 1, "\u{FF61}": 2 };
    expect(canonicalize(value)).toBe('{"\u{1F600}":1,"\u{FF61}":2}');
    expect(unwrap(jcsSha256Hex(value))).toBe(
      "88954a1c73177a8dd8c59046f982693fbcfb908d08fbe5b06e82868052487feb",
    );
  });

  it("emits 64 lowercase hex", () => {
    const digest = unwrap(jcsSha256Hex({ a: 1 }));
    expect(digest).toMatch(/^[0-9a-f]{64}$/);
  });

  it("digests excluding a field (chunk_digest / bundle digest)", () => {
    const chunk = { chunk_digest: "deadbeef", a: 1, z: 2 };
    expect(unwrap(jcsSha256HexExcluding(chunk, "chunk_digest"))).toBe(
      "99168216144c7fed5d4c54916cf98d9c66096280c04a499822a99b6658bd177a",
    );
  });

  it("rejects non-JSON values as INVALID_INPUT", () => {
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    const reject = unwrapReject(jcsSha256Hex(circular));
    expect(reject.code).toBe("INVALID_INPUT");
  });
});
