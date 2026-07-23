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
});
