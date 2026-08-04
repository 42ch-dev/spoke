/**
 * libp2p Noise identity payload gates (Task 3, connect-ts-noise-stack) —
 * frozen contract `.mstar/specs/noise-xx-libp2p-contract.md` §4:
 *
 *   - §4.2 domain-separated Ed25519 signature over
 *     `"noise-libp2p-static-key:" || static_public_x25519`,
 *   - §4.3 the `NoiseHandshakePayload` protobuf (identity_key /
 *     identity_sig / optional extensions) and the libp2p `PublicKey`
 *     protobuf carried in `identity_key`.
 *
 * Fixtures:
 *   - identity seed = the golden-hello seed (`tests/fixtures/golden-hello.json`),
 *     so the derived Ed25519 public key is the package-pinned
 *     `79b5562e…9664` / peer_id `12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf`.
 *   - static public = X25519 public of the cacophony initiator static
 *     private key (same key used by `tests/noise/xx.test.ts`).
 *   - expected signature computed once with `@noble/ed25519` (RFC 8032
 *     deterministic): pins the domain string + byte concatenation; any
 *     future change to `STATIC_KEY_DOMAIN` or the preimage layout breaks
 *     this fixture (interop proof lives in Task 4's rust transcript).
 */
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

// ── Fixtures ────────────────────────────────────────────────────────────────

/** golden-hello.json seed (package-pinned identity). */
const IDENTITY_SEED = fromHex(
  "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
);
/** Its Ed25519 public key (pinned in golden-hello.json). */
const IDENTITY_PUBLIC = fromHex(
  "79b5562e8fe654f94078b112e8a98ba7901f853ae695bed7e0e3910bad049664",
);
/** X25519 public of the cacophony initiator static key (xx.test.ts). */
const STATIC_PUBLIC = fromHex(
  "6bc3822a2aa7f4e6981d6538692b3cdf3e6df9eea6ed269eb41d93c22757b75a",
);
/** Ed25519 signature of the golden identity over the domain preimage. */
const EXPECTED_SIGNATURE = fromHex(
  "cf05b6ba239566483c7106cb7a8bf1137e3fcc12d9f003ddb94f1211d5c7ef28" +
    "fda4f0efbb005d0411eedaf38cb7067157d3f388a6509811395ccbf8e8424405",
);
/** ASCII "noise-libp2p-static-key:" (no NUL) — pinned byte layout. */
const DOMAIN_HEX = "6e6f6973652d6c69627032702d7374617469632d6b65793a";

// ── §4.2 domain string ──────────────────────────────────────────────────────

describe("STATIC_KEY_DOMAIN", () => {
  it("is the frozen ASCII domain string (no NUL)", () => {
    expect(new TextEncoder().encode(STATIC_KEY_DOMAIN)).toEqual(fromHex(DOMAIN_HEX));
  });
});

// ── §4.2 sign / verify ──────────────────────────────────────────────────────

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

// ── §4.3 libp2p PublicKey protobuf (identity_key) ───────────────────────────

describe("encodeIdentityPublicKey / decodeIdentityPublicKey", () => {
  it("encodes the libp2p PublicKey protobuf for Ed25519 (36 bytes)", () => {
    // field 1 varint Type = 1 (Ed25519): 08 01; field 2 bytes Data: 12 20 || key
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
    // RSA (Type = 0): 08 00
    const rsa = Uint8Array.of(0x08, 0x00, 0x12, 0x20, ...IDENTITY_PUBLIC);
    expect(decodeIdentityPublicKey(rsa)).toBeUndefined();
  });

  it("rejects malformed or truncated encodings", () => {
    expect(decodeIdentityPublicKey(new Uint8Array(0))).toBeUndefined();
    expect(decodeIdentityPublicKey(IDENTITY_PUBLIC)).toBeUndefined(); // raw key ≠ protobuf
    expect(
      decodeIdentityPublicKey(encodeIdentityPublicKey(IDENTITY_PUBLIC).slice(0, 35)),
    ).toBeUndefined();
  });

  it("rejects an out-of-contract key length on encode", () => {
    expect(() => encodeIdentityPublicKey(new Uint8Array(31))).toThrow();
  });
});

// ── §4.3 NoiseHandshakePayload protobuf ─────────────────────────────────────

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
    // 0a 24 (field 1, 36-byte PublicKey) || 12 40 (field 2, 64-byte sig)
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
    // append field 3 (bytes "abc") — 1a 03 61 62 63
    const withUnknown = fromHex(toHex(base) + "1a03616263");
    const decoded = decodeHandshakePayload(withUnknown);
    expect(decoded.identityKey).toEqual(encodeIdentityPublicKey(IDENTITY_PUBLIC));
    expect(decoded.identitySig).toEqual(EXPECTED_SIGNATURE);
  });

  it("skips unknown fields inside NoiseExtensions", () => {
    // payload = identity_key + identity_sig, then field 4 (extensions) whose
    // inner message is `0a 02 01 02` (field 1, certhash 0102) plus unknown
    // field 3 (bytes 01) — `22 07 0a 02 01 02 1a 01 01`.
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
    expect(() => decodeHandshakePayload(fromHex("0a"))).toThrow(); // truncated length
    expect(() => decodeHandshakePayload(fromHex("0a0501"))).toThrow(); // declared > available
    expect(() => decodeHandshakePayload(fromHex("1b01"))).toThrow(); // group wire type
    expect(() => decodeHandshakePayload(fromHex("ffffffffffffffffffffffff"))).toThrow(); // varint overrun
  });
});
