import { describe, expect, it } from "vitest";

import { isAllowlisted } from "../src/core/allowlist.js";

describe("allowlist (port of allowlist.rs)", () => {
  it("accepts listed peers", () => {
    const allowlist = ["peer-a", "peer-b"];
    expect(isAllowlisted(allowlist, "peer-a")).toBe(true);
    expect(isAllowlisted(allowlist, "peer-b")).toBe(true);
  });

  it("rejects unlisted peers", () => {
    const allowlist = ["peer-a"];
    expect(isAllowlisted(allowlist, "peer-c")).toBe(false);
  });

  it("rejects everyone on an empty allowlist (fail-closed)", () => {
    expect(isAllowlisted([], "peer-a")).toBe(false);
  });
});
