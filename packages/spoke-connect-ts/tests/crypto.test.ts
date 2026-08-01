import { beforeAll, describe, expect, it } from "vitest";

import {
  base64UrlDecode,
  base64UrlEncode,
  ed25519PubkeyToSpki,
  ed25519SeedToPkcs8,
  getPublicKeyEd25519,
  signEd25519Noble,
  verifyEd25519Noble,
  webcryptoEd25519Available,
} from "../src/crypto.js";
import { GOLDEN_PUBKEY, GOLDEN_SEED } from "../src/golden.js";
import { toHex } from "./hex.js";

describe("base64url (no padding)", () => {
  it("encodes without '=' padding or url-unsafe characters", () => {
    // 0xfb 0xff → base64 "+/8=" → base64url "-_8" (padding stripped).
    const encoded = base64UrlEncode(Uint8Array.of(0xfb, 0xff));
    expect(encoded).toBe("-_8");
    expect(encoded).not.toMatch(/[+/=]/);
  });

  it("round-trips arbitrary bytes", () => {
    const bytes = new Uint8Array(64).map((_, i) => i * 7 + 1);
    expect(base64UrlDecode(base64UrlEncode(bytes))).toEqual(bytes);
  });

  it("accepts padded input on decode", () => {
    expect(base64UrlDecode("-_8").length).toBe(2);
    expect(base64UrlDecode("-_8=")).toEqual(Uint8Array.of(0xfb, 0xff));
  });
});

describe("DER wrapping (RFC 8410)", () => {
  it("wraps a 32-byte seed in PKCS8 (16-byte header)", () => {
    const pkcs8 = ed25519SeedToPkcs8(GOLDEN_SEED);
    expect(pkcs8.length).toBe(48);
    expect(toHex(pkcs8.subarray(0, 16))).toBe(
      "302e020100300506032b657004220420",
    );
    expect(pkcs8.subarray(16)).toEqual(GOLDEN_SEED);
  });

  it("wraps a 32-byte public key in SPKI (12-byte header)", () => {
    const spki = ed25519PubkeyToSpki(GOLDEN_PUBKEY);
    expect(spki.length).toBe(44);
    expect(toHex(spki.subarray(0, 12))).toBe("302a300506032b6570032100");
    expect(spki.subarray(12)).toEqual(GOLDEN_PUBKEY);
  });
});

describe("getPublicKeyEd25519", () => {
  it("derives the golden public key from the golden seed", () => {
    expect(toHex(getPublicKeyEd25519(GOLDEN_SEED))).toBe(
      toHex(GOLDEN_PUBKEY),
    );
  });
});

describe("@noble/ed25519 path (fallback, always exercised)", () => {
  const message = new TextEncoder().encode("hello over the wire");

  it("sign → verify round-trip", () => {
    const pubkey = getPublicKeyEd25519(GOLDEN_SEED);
    const sig = signEd25519Noble(GOLDEN_SEED, message);
    expect(sig.length).toBe(64);
    expect(verifyEd25519Noble(pubkey, message, sig)).toBe(true);
  });

  it("rejects a tampered message", () => {
    const pubkey = getPublicKeyEd25519(GOLDEN_SEED);
    const sig = signEd25519Noble(GOLDEN_SEED, message);
    const tampered = new TextEncoder().encode("hello over the wire!");
    expect(verifyEd25519Noble(pubkey, tampered, sig)).toBe(false);
  });
});

describe("WebCrypto Ed25519 path (when the runtime supports it)", () => {
  let webcryptoOk = false;

  beforeAll(async () => {
    webcryptoOk = await webcryptoEd25519Available();
  });

  it("probe is true iff an Ed25519 importKey succeeds on this runtime", async () => {
    // `importKey` presence is not the contract — it exists yet rejects
    // Ed25519 on Node 20.0–20.18 / 21 / 22.0–22.3. Assert the probe's
    // contract directly: true iff an actual Ed25519 importKey succeeds.
    const s = globalThis.crypto?.subtle;
    let importSucceeds = false;
    if (s && typeof s.importKey === "function") {
      try {
        await s.importKey(
          "pkcs8",
          ed25519SeedToPkcs8(GOLDEN_SEED) as unknown as BufferSource,
          { name: "Ed25519" },
          false,
          ["sign"],
        );
        importSucceeds = true;
      } catch {
        importSucceeds = false;
      }
    }
    expect(webcryptoOk).toBe(importSucceeds);
  });

  it("sign → verify round-trip via crypto.subtle directly", async (ctx) => {
    if (!webcryptoOk) ctx.skip();
    // BufferSource requires ArrayBuffer-backed views under TS 5.7+ generics;
    // these test buffers are ArrayBuffer-backed by construction.
    const s = globalThis.crypto.subtle;
    const key = await s.importKey(
      "pkcs8",
      ed25519SeedToPkcs8(GOLDEN_SEED) as unknown as BufferSource,
      { name: "Ed25519" },
      false,
      ["sign"],
    );
    const message = new TextEncoder().encode("webcrypto message");
    const sig = new Uint8Array(
      await s.sign(
        { name: "Ed25519" },
        key,
        message as unknown as BufferSource,
      ),
    );
    expect(sig.length).toBe(64);

    const verifyKey = await s.importKey(
      "spki",
      ed25519PubkeyToSpki(GOLDEN_PUBKEY) as unknown as BufferSource,
      { name: "Ed25519" },
      false,
      ["verify"],
    );
    expect(
      await s.verify(
        { name: "Ed25519" },
        verifyKey,
        sig as unknown as BufferSource,
        message as unknown as BufferSource,
      ),
    ).toBe(true);
  });
});
