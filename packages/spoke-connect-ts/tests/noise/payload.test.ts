import { describe, expect, it } from "vitest";

import {
  STATIC_KEY_DOMAIN,
  decodeHandshakePayload,
  decodeIdentityPublicKey,
  encodeHandshakePayload,
  encodeIdentityPublicKey,
  signNoiseStaticKey,
  verifyNoiseStaticKey,
} from "../../src/noise/payload.js";
import { fromHex, toHex } from "../hex.js";

const IDENTITY_SEED = fromHex(
  "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
);
const IDENTITY_PUBLIC = fromHex(
  "79b5562e8fe654f94078b112e8a98ba7901f853ae695bed7e0e3910bad049664",
);
const STATIC_PUBLIC = fromHex(
  "6bc3822a2aa7f4e6981d6538692b3cdf3e6df9eea6ed269eb41d93c22757b75a",
);
const EXPECTED_SIGNATURE = fromHex(
  "cf05b6ba239566483c7106cb7a8bf1137e3fcc12d9f003ddb94f1211d5c7ef28" +
    "fda4f0efbb005d0411eedaf38cb7067157d3f388a6509811395ccbf8e8424405",
);
const DOMAIN_HEX = "6e6f6973652d6c69627032702d7374617469632d6b65793a";

describe("STATIC_KEY_DOMAIN", () => {
  it("is the frozen ASCII domain string (no NUL)", () => {
    expect(new TextEncoder().encode(STATIC_KEY_DOMAIN)).toEqual(fromHex(DOMAIN_HEX));
  });
});

describe("signNoiseStaticKey / verifyNoiseStaticKey", () => {
  it("produces the deterministic signature fixture over the domain preimage", async () => {
    const signature = await signNoiseStaticKey(IDENTITY_SEED, STATIC_PUBLIC);
    expect(signature).toHaveLength(64);
    expect(toHex(signature)).toBe(toHex(EXPECTED_SIGNATURE));
  });

  it("verifies a signature it produced", async () => {
    const signature = await signNoiseStaticKey(IDENTITY_SEED, STATIC_PUBLIC);
    await expect(
      verifyNoiseStaticKey(IDENTITY_PUBLIC, STATIC_PUBLIC, signature),
    ).resolves.toBe(true);
  });

  it("rejects a tampered signature", async () => {
    const signature = await signNoiseStaticKey(IDENTITY_SEED, STATIC_PUBLIC);
    signature[0] ^= 0x01;
    await expect(
      verifyNoiseStaticKey(IDENTITY_PUBLIC, STATIC_PUBLIC, signature),
    ).resolves.toBe(false);
  });

  it("rejects a signature over a different static key", async () => {
    const signature = await signNoiseStaticKey(IDENTITY_SEED, STATIC_PUBLIC);
    const otherStatic = new Uint8Array(STATIC_PUBLIC);
    otherStatic[31] ^= 0x01;
    await expect(
      verifyNoiseStaticKey(IDENTITY_PUBLIC, otherStatic, signature),
    ).resolves.toBe(false);
  });

  it("rejects a signature from a different identity", async () => {
    const signature = await signNoiseStaticKey(IDENTITY_SEED, STATIC_PUBLIC);
    const otherIdentity = new Uint8Array(IDENTITY_PUBLIC);
    otherIdentity[0] ^= 0x01;
    await expect(
      verifyNoiseStaticKey(otherIdentity, STATIC_PUBLIC, signature),
    ).resolves.toBe(false);
  });

  it("rejects a malformed signature length (false, not throw)", async () => {
    await expect(
      verifyNoiseStaticKey(IDENTITY_PUBLIC, STATIC_PUBLIC, new Uint8Array(63)),
    ).resolves.toBe(false);
  });
});

describe("encodeIdentityPublicKey / decodeIdentityPublicKey", () => {
  it("encodes the libp2p PublicKey protobuf for Ed25519 (36 bytes)", () => {
    expect(toHex(encodeIdentityPublicKey(IDENTITY_PUBLIC))).toBe(
      "08011220" + toHex(IDENTITY_PUBLIC),
    );
    expect(encodeIdentityPublicKey(IDENTITY_PUBLIC)).toHaveLength(36);
  });

  it("round-trips the raw key", () => {
    expect(decodeIdentityPublicKey(encodeIdentityPublicKey(IDENTITY_PUBLIC))).toEqual(
      IDENTITY_PUBLIC,
    );
  });

  it("rejects non-Ed25519 key types", () => {
    const rsa = Uint8Array.of(0x08, 0x00, 0x12, 0x20, ...IDENTITY_PUBLIC);
    expect(decodeIdentityPublicKey(rsa)).toBeUndefined();
  });

  it("rejects malformed or truncated encodings", () => {
    expect(decodeIdentityPublicKey(new Uint8Array(0))).toBeUndefined();
    expect(decodeIdentityPublicKey(IDENTITY_PUBLIC)).toBeUndefined();
    expect(
      decodeIdentityPublicKey(encodeIdentityPublicKey(IDENTITY_PUBLIC).slice(0, 35)),
    ).toBeUndefined();
  });

  it("rejects an out-of-contract key length on encode", () => {
    expect(() => encodeIdentityPublicKey(new Uint8Array(31))).toThrow();
  });
});

describe("encodeHandshakePayload / decodeHandshakePayload", () => {
  it("encodes the default payload as zero bytes (flight 1)", () => {
    expect(encodeHandshakePayload({})).toHaveLength(0);
    expect(encodeHandshakePayload({ identityKey: undefined })).toHaveLength(0);
  });

  it("encodes identity_key + identity_sig to the pinned byte layout", () => {
    const encoded = encodeHandshakePayload({
      identityKey: encodeIdentityPublicKey(IDENTITY_PUBLIC),
      identitySig: EXPECTED_SIGNATURE,
    });
    expect(toHex(encoded)).toBe(
      "0a2408011220" + toHex(IDENTITY_PUBLIC) + "1240" + toHex(EXPECTED_SIGNATURE),
    );
    expect(encoded).toHaveLength(104);
  });

  it("round-trips a full payload", () => {
    const payload = {
      identityKey: encodeIdentityPublicKey(IDENTITY_PUBLIC),
      identitySig: EXPECTED_SIGNATURE,
    };
    expect(decodeHandshakePayload(encodeHandshakePayload(payload))).toEqual(payload);
  });

  it("round-trips NoiseExtensions (stream_muxers + webtransport_certhashes)", () => {
    const payload = {
      identityKey: encodeIdentityPublicKey(IDENTITY_PUBLIC),
      identitySig: EXPECTED_SIGNATURE,
      extensions: {
        streamMuxers: ["yamux"],
        webtransportCerthashes: [Uint8Array.of(0x01, 0x02)],
      },
    };
    const decoded = decodeHandshakePayload(encodeHandshakePayload(payload));
    expect(decoded.identityKey).toEqual(payload.identityKey);
    expect(decoded.identitySig).toEqual(payload.identitySig);
    expect(decoded.extensions).toEqual(payload.extensions);
  });

  it("skips unknown payload fields when decoding", () => {
    const base = encodeHandshakePayload({
      identityKey: encodeIdentityPublicKey(IDENTITY_PUBLIC),
      identitySig: EXPECTED_SIGNATURE,
    });
    const withUnknown = fromHex(toHex(base) + "1a03616263");
    const decoded = decodeHandshakePayload(withUnknown);
    expect(decoded.identityKey).toEqual(encodeIdentityPublicKey(IDENTITY_PUBLIC));
    expect(decoded.identitySig).toEqual(EXPECTED_SIGNATURE);
  });

  it("skips unknown fields inside NoiseExtensions", () => {
    const base = encodeHandshakePayload({
      identityKey: encodeIdentityPublicKey(IDENTITY_PUBLIC),
      identitySig: EXPECTED_SIGNATURE,
    });
    const withUnknown = fromHex(toHex(base) + "22070a0201021a0101");
    const decoded = decodeHandshakePayload(withUnknown);
    expect(decoded.extensions).toEqual({
      webtransportCerthashes: [Uint8Array.of(1, 2)],
    });
    expect(decoded.identityKey).toEqual(encodeIdentityPublicKey(IDENTITY_PUBLIC));
    expect(decoded.identitySig).toEqual(EXPECTED_SIGNATURE);
  });

  it("rejects malformed protobuf", () => {
    expect(() => decodeHandshakePayload(fromHex("0a"))).toThrow();
    expect(() => decodeHandshakePayload(fromHex("0a0501"))).toThrow();
    expect(() => decodeHandshakePayload(fromHex("1b01"))).toThrow();
    expect(() => decodeHandshakePayload(fromHex("ffffffffffffffffffffffff"))).toThrow();
  });
});
