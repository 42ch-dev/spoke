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
    // Same pins as crates/spoke-operations/src/memory/digests.rs; SHA-256
    // produced independently via `shasum -a 256` over the JCS bytes.
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

  it("sorts non-BMP member names by UTF-16 code units like Rust", () => {
    // RFC 8785 §3.2.3 sorts member names by UTF-16 code units. U+1F600
    // encodes as the surrogate pair D83D DE00, so it orders BEFORE the BMP
    // key U+FF61 — the opposite of Unicode-scalar order (which would put
    // U+FF61 first). The Rust test pins the same canonical bytes and digest.
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
      // shasum -a 256 over '{"a":1,"z":2}'
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
