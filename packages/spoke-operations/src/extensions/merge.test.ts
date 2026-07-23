import { describe, expect, it } from "vitest";

import {
  mergeExtensionMaps,
  preserveExtensionMaps,
} from "./merge.js";

describe("mergeExtensionMaps", () => {
  it("preserves unknown namespaces from both inputs", () => {
    const base = {
      nexus: { world_id: "w1" },
      creader: { book_id: "b1" },
    };
    const overlay = {
      nexus: { editor: "v2" },
    };

    const result = mergeExtensionMaps(base, overlay);

    expect(result.nexus).toEqual({ world_id: "w1", editor: "v2" });
    expect(result.creader).toEqual({ book_id: "b1" });
  });

  it("lets overlay win on scalar conflicts", () => {
    const base = { nexus: { mode: "draft", keep: true } };
    const overlay = { nexus: { mode: "published" } };

    const result = mergeExtensionMaps(base, overlay);

    expect(result.nexus).toEqual({ mode: "published", keep: true });
  });

  it("keeps empty namespace objects", () => {
    const base = { nexus: {} };
    const overlay = { creader: { flag: true } };

    const result = mergeExtensionMaps(base, overlay);

    expect(result.nexus).toEqual({});
    expect(result.creader).toEqual({ flag: true });
  });

  it("does not mutate inputs", () => {
    const base = { nexus: { a: 1 } };
    const overlay = { nexus: { b: 2 } };
    const baseCopy = structuredClone(base);
    const overlayCopy = structuredClone(overlay);

    mergeExtensionMaps(base, overlay);

    expect(base).toEqual(baseCopy);
    expect(overlay).toEqual(overlayCopy);
  });

  it("does not alias nested objects from inputs", () => {
    const base = { nexus: { nested: { count: 1 } } };
    const overlay = { nexus: { tags: ["draft"] } };

    const result = mergeExtensionMaps(base, overlay);

    result.nexus.nested.count = 99;
    (result.nexus.tags as string[]).push("published");

    expect(base.nexus.nested.count).toBe(1);
    expect(base.nexus).not.toHaveProperty("tags");
    expect(overlay.nexus.tags).toEqual(["draft"]);
  });
});

describe("preserveExtensionMaps", () => {
  it("retains unknown keys from source while target wins known keys", () => {
    const source = {
      nexus: { legacy: "keep", mode: "old" },
      creader: { only_source: true },
    };
    const target = {
      nexus: { mode: "new" },
    };

    const result = preserveExtensionMaps(source, target);

    expect(result.nexus).toEqual({ legacy: "keep", mode: "new" });
    expect(result.creader).toEqual({ only_source: true });
  });

  it("does not delete sibling namespaces when overlaying one namespace", () => {
    const source = {
      nexus: { a: 1 },
      creader: { b: 2 },
    };
    const target = {
      nexus: { c: 3 },
    };

    const result = preserveExtensionMaps(source, target);

    expect(result.nexus).toEqual({ a: 1, c: 3 });
    expect(result.creader).toEqual({ b: 2 });
  });

  it("does not alias nested objects from inputs", () => {
    const source = { nexus: { meta: { legacy: true } } };
    const target = { nexus: { meta: { mode: "new" } } };

    const result = preserveExtensionMaps(source, target);

    result.nexus.meta.legacy = false;
    result.nexus.meta.mode = "edited";

    expect(source.nexus.meta).toEqual({ legacy: true });
    expect(target.nexus.meta).toEqual({ mode: "new" });
  });
});
