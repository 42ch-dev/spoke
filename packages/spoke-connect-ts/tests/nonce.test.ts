import { describe, expect, it } from "vitest";

import { generateNonce, NonceStore } from "../src/core/nonce.js";

describe("NonceStore (port of nonce.rs)", () => {
  it("treats a nonce as single-use per peer", () => {
    const store = new NonceStore();
    expect(store.checkAndRecord("peer-a", "nonce-1")).toBe(true);
    expect(store.checkAndRecord("peer-a", "nonce-1")).toBe(false);
    // A different nonce from the same peer is fresh.
    expect(store.checkAndRecord("peer-a", "nonce-2")).toBe(true);
  });

  it("allows the same nonce from different peers (scoping is per sender)", () => {
    const store = new NonceStore();
    expect(store.checkAndRecord("peer-a", "shared-nonce")).toBe(true);
    expect(store.checkAndRecord("peer-b", "shared-nonce")).toBe(true);
  });

  it("accepts everything on a fresh store", () => {
    const store = new NonceStore();
    expect(store.checkAndRecord("peer-a", "n-1")).toBe(true);
    expect(store.checkAndRecord("peer-b", "n-1")).toBe(true);
    expect(store.checkAndRecord("peer-b", "n-2")).toBe(true);
  });
});

describe("generateNonce", () => {
  it("meets the wire floor and is fresh per call", () => {
    const a = generateNonce();
    const b = generateNonce();
    expect(a.length).toBeGreaterThanOrEqual(16);
    expect(b.length).toBeGreaterThanOrEqual(16);
    expect(a).not.toBe(b);
  });
});
