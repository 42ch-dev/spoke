import { describe, expect, it } from "vitest";

import {
  mergeExtensionMaps,
  preserveExtensionMaps,
  mergeModuleMaps,
  preserveModuleMaps,
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

describe("mergeModuleMaps", () => {
  it("deep-merges object-valued namespaces (activation shape)", () => {
    const base = { activation: { state: "idle", fuel: 10 } };
    const overlay = { activation: { state: "active" } };

    const result = mergeModuleMaps(base, overlay);

    expect(result.activation).toEqual({ state: "active", fuel: 10 });
  });

  it("replaces array-valued namespaces instead of element-merging (placement shape)", () => {
    const base = {
      placement: [{ entry_id: "a", position_hint: 0 }],
    };
    const overlay = {
      placement: [{ entry_id: "b", position_hint: 1 }],
    };

    const result = mergeModuleMaps(base, overlay);

    expect(result.placement).toEqual([{ entry_id: "b", position_hint: 1 }]);
  });

  it("preserves unknown namespaces from both inputs (object and array)", () => {
    const base = {
      activation: { state: "idle" },
      custom_obj: { k: 1 },
    };
    const overlay = {
      placement: [{ p: 1 }],
      custom_arr: [9],
    };

    const result = mergeModuleMaps(base, overlay);

    expect(result.activation).toEqual({ state: "idle" });
    expect(result.custom_obj).toEqual({ k: 1 });
    expect(result.placement).toEqual([{ p: 1 }]);
    expect(result.custom_arr).toEqual([9]);
  });

  it("treats empty maps and empty namespaces as valid", () => {
    expect(mergeModuleMaps({}, {})).toEqual({});

    const result = mergeModuleMaps(
      { activation: {} },
      { placement: [] },
    );

    expect(result.activation).toEqual({});
    expect(result.placement).toEqual([]);
  });

  it("does not alias arrays cloned from inputs", () => {
    const base = { placement: [{ entry_id: "a" }] };
    const overlay = { activation: { state: "x" } };

    const result = mergeModuleMaps(base, overlay);

    (result.placement as unknown[]).push({ entry_id: "z" });

    expect(base.placement).toEqual([{ entry_id: "a" }]);
  });
});

describe("preserveModuleMaps", () => {
  it("retains unknown namespaces from source (object and array) while target wins known keys", () => {
    const source = {
      activation: { legacy: true, mode: "old" },
      placement: [{ entry_id: "old" }],
      custom: { only_source: 1 },
    };
    const target = {
      activation: { mode: "new" },
    };

    const result = preserveModuleMaps(source, target);

    expect(result.activation).toEqual({ legacy: true, mode: "new" });
    expect(result.placement).toEqual([{ entry_id: "old" }]);
    expect(result.custom).toEqual({ only_source: 1 });
  });

  it("does not delete sibling namespaces when overlaying one", () => {
    const source = {
      activation: { a: 1 },
      placement: [{ p: 1 }],
    };
    const target = { activation: { c: 3 } };

    const result = preserveModuleMaps(source, target);

    expect(result.activation).toEqual({ a: 1, c: 3 });
    expect(result.placement).toEqual([{ p: 1 }]);
  });

  it("lets target replace an array-valued namespace it also owns", () => {
    const source = { placement: [{ entry_id: "old" }] };
    const target = { placement: [{ entry_id: "new" }] };

    const result = preserveModuleMaps(source, target);

    expect(result.placement).toEqual([{ entry_id: "new" }]);
  });
});
