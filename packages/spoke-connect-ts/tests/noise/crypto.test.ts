/**
 * RFC-vector gates for the Noise crypto primitives (Task 1 of
 * connect-ts-noise-stack).
 *
 * Every primitive is pinned against an RFC vector:
 *   - X25519 DH:      RFC 7748 §6.1 (Alice/Bob key pair + shared secret)
 *   - HKDF-SHA256:    RFC 5869 §A.1–A.3 (including the empty-info case that
 *                     matches the Noise HKDF shape)
 *   - ChaCha20-Poly1305: RFC 8439 §2.8.2 AEAD test vector
 *
 * The Noise-constant-shaped wrappers (`hkdf`, `encrypt`/`decrypt`) carry
 * fixed vectors generated with an independent OpenSSL-backed oracle
 * (`node:crypto` — NOT noble), so the wrapper semantics are pinned by a
 * second implementation. Generation script and rationale: Task 1 report.
 *
 * Noise AEAD nonce framing (per Noise spec + frozen contract §3): 96-bit
 * nonce = 4 zero bytes || 64-bit little-endian counter.
 */
import { describe, expect, it } from "vitest";

import {
  chacha20Poly1305Open,
  chacha20Poly1305Seal,
  decrypt,
  dh,
  encrypt,
  getPublicKey,
  hkdf,
  hkdfSha256,
} from "../../src/noise/crypto.js";
import { fromHex, toHex } from "../hex.js";

// ── RFC 7748 §6.1 test vectors ─────────────────────────────────────────────

const RFC7748 = {
  alicePrivate: fromHex(
    "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
  ),
  alicePublic: fromHex(
    "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a",
  ),
  bobPrivate: fromHex(
    "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
  ),
  bobPublic: fromHex(
    "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f",
  ),
  shared: fromHex(
    "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742",
  ),
};

describe("X25519 DH (RFC 7748 §6.1)", () => {
  it("derives the Alice public key from her private key", () => {
    expect(toHex(getPublicKey(RFC7748.alicePrivate))).toBe(
      toHex(RFC7748.alicePublic),
    );
  });

  it("derives the Bob public key from his private key", () => {
    expect(toHex(getPublicKey(RFC7748.bobPrivate))).toBe(toHex(RFC7748.bobPublic));
  });

  it("Alice dh(private, Bob public) matches the RFC shared secret", () => {
    expect(toHex(dh(RFC7748.alicePrivate, RFC7748.bobPublic))).toBe(
      toHex(RFC7748.shared),
    );
  });

  it("Bob dh(private, Alice public) matches the RFC shared secret", () => {
    expect(toHex(dh(RFC7748.bobPrivate, RFC7748.alicePublic))).toBe(
      toHex(RFC7748.shared),
    );
  });

  it("rejects non-32-byte keys", () => {
    expect(() => dh(new Uint8Array(31), RFC7748.bobPublic)).toThrow();
    expect(() => dh(RFC7748.alicePrivate, new Uint8Array(33))).toThrow();
  });
});

// ── RFC 5869 §A.1–A.3 test vectors ─────────────────────────────────────────

// A.1: 22×0x0b IKM, 13-byte salt 00..0c, 10-byte info f0..f9, L=42.
// A.2: 80-byte IKM 00..4f, 80-byte salt 60..af, 80-byte info b0..ff, L=82.
// A.3: 22×0x0b IKM, empty salt, empty info, L=42 (the Noise-shape case).
const RFC5869 = {
  a1: {
    ikm: new Uint8Array(22).fill(0x0b),
    salt: Uint8Array.from({ length: 13 }, (_, i) => i),
    info: Uint8Array.from({ length: 10 }, (_, i) => 0xf0 + i),
    okm: "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865",
  },
  a2: {
    ikm: Uint8Array.from({ length: 80 }, (_, i) => i),
    salt: Uint8Array.from({ length: 80 }, (_, i) => 0x60 + i),
    info: Uint8Array.from({ length: 80 }, (_, i) => 0xb0 + i),
    okm: "b11e398dc80327a1c8e7f78c596a49344f012eda2d4efad8a050cc4c19afa97c59045a99cac7827271cb41c65e590e09da3275600c2f09b8367793a9aca3db71cc30c58179ec3e87c14c01d5c1f3434f1d87",
  },
  a3: {
    ikm: new Uint8Array(22).fill(0x0b),
    salt: new Uint8Array(0),
    info: new Uint8Array(0),
    okm: "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8",
  },
};

describe("HKDF-SHA256 (RFC 5869)", () => {
  it("matches RFC 5869 §A.1 (42-byte OKM)", () => {
    const { ikm, salt, info, okm } = RFC5869.a1;
    expect(toHex(hkdfSha256(salt, ikm, info, 42))).toBe(okm);
  });

  it("matches RFC 5869 §A.2 (82-byte OKM)", () => {
    const { ikm, salt, info, okm } = RFC5869.a2;
    expect(toHex(hkdfSha256(salt, ikm, info, 82))).toBe(okm);
  });

  it("matches RFC 5869 §A.3 (empty salt + info)", () => {
    const { ikm, salt, info, okm } = RFC5869.a3;
    expect(toHex(hkdfSha256(salt, ikm, info, 42))).toBe(okm);
  });
});

// ── Noise hkdf(ck, ikm) → { chainingKey, key } ────────────────────────────
//
// Noise HKDF (Noise spec §5.2, hash = SHA-256): two 32-byte outputs from
// (chaining_key, input_key_material) with empty info — exactly
// HKDF-SHA256(Extract(ck, ikm), Expand("", 64)) split at byte 32.
//
// Fixed vector: ck = SHA-256("Noise_XX_25519_ChaChaPoly_SHA256"),
// ikm = 32×0x11. Expected values generated with `node:crypto` hkdfSync
// (OpenSSL) and cross-checked against noble; see Task 1 report.

const NOISE_HKDF_CK = "f3d15e6108ed9556171207baa58f97d29a13c6be40595166066e2e0958dc002d";
const NOISE_HKDF_NEW_CK =
  "4e2b33a6ebe06dd7e806e6bf3010ac6574b6be37c8b326528289b4882fb6819a";
const NOISE_HKDF_KEY =
  "9e2484d576475899f301581bb872772582fd5852a60fcec760466f6fa6dd6a36";

describe("Noise hkdf(chainingKey, inputKeyMaterial)", () => {
  const ck = fromHex(NOISE_HKDF_CK);
  const ikm = new Uint8Array(32).fill(0x11);

  it("returns two 32-byte outputs (new chaining key + cipher key)", () => {
    const { chainingKey, key } = hkdf(ck, ikm);
    expect(chainingKey).toHaveLength(32);
    expect(key).toHaveLength(32);
  });

  it("matches the independent OpenSSL oracle vector", () => {
    const { chainingKey, key } = hkdf(ck, ikm);
    expect(toHex(chainingKey)).toBe(NOISE_HKDF_NEW_CK);
    expect(toHex(key)).toBe(NOISE_HKDF_KEY);
  });

  it("is consistent with the general HKDF-SHA256 (empty info, L=64)", () => {
    const { chainingKey, key } = hkdf(ck, ikm);
    const okm = hkdfSha256(ck, ikm, new Uint8Array(0), 64);
    expect(toHex(chainingKey)).toBe(toHex(okm.subarray(0, 32)));
    expect(toHex(key)).toBe(toHex(okm.subarray(32, 64)));
  });
});

// ── RFC 8439 §2.8.2 AEAD test vector ───────────────────────────────────────

// Key 80..9f, nonce 07 00 00 00 40 41 42 43 44 45 46 47 (32-bit fixed part
// 7 || 64-bit IV), AAD 50 51 52 53 c0..c7, plaintext = RFC "sunscreen" text.
const RFC8439 = {
  key: Uint8Array.from({ length: 32 }, (_, i) => 0x80 + i),
  nonce12: fromHex("070000004041424344454647"),
  aad: fromHex("50515253c0c1c2c3c4c5c6c7"),
  plaintext: new TextEncoder().encode(
    "Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.",
  ),
  ciphertext:
    "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b6116",
  tag: "1ae10b594f09e26a7e902ecbd0600691",
};

describe("ChaCha20-Poly1305 (RFC 8439 §2.8.2)", () => {
  it("seals the RFC plaintext to the expected ciphertext + tag", () => {
    const { key, nonce12, aad, plaintext, ciphertext, tag } = RFC8439;
    const sealed = chacha20Poly1305Seal(key, nonce12, aad, plaintext);
    expect(sealed).toHaveLength(plaintext.length + 16);
    expect(toHex(sealed.subarray(0, -16))).toBe(ciphertext);
    expect(toHex(sealed.subarray(-16))).toBe(tag);
  });

  it("opens the RFC ciphertext back to the plaintext", () => {
    const { key, nonce12, aad, plaintext, ciphertext, tag } = RFC8439;
    const opened = chacha20Poly1305Open(
      key,
      nonce12,
      aad,
      fromHex(ciphertext + tag),
    );
    expect(opened).not.toBeNull();
    expect(toHex(opened!)).toBe(toHex(plaintext));
  });

  it("rejects a tampered ciphertext byte", () => {
    const { key, nonce12, aad, ciphertext, tag } = RFC8439;
    const tampered = fromHex(ciphertext + tag);
    tampered[0] ^= 0x01;
    expect(chacha20Poly1305Open(key, nonce12, aad, tampered)).toBeNull();
  });

  it("rejects a wrong AAD", () => {
    const { key, nonce12, ciphertext, tag } = RFC8439;
    const wrongAad = new Uint8Array([0x00]);
    expect(
      chacha20Poly1305Open(key, nonce12, wrongAad, fromHex(ciphertext + tag)),
    ).toBeNull();
  });

  it("rejects a wrong key", () => {
    const { key, nonce12, aad, ciphertext, tag } = RFC8439;
    const wrongKey = new Uint8Array(key);
    wrongKey[0] ^= 0x01;
    expect(
      chacha20Poly1305Open(wrongKey, nonce12, aad, fromHex(ciphertext + tag)),
    ).toBeNull();
  });
});

// ── Noise encrypt/decrypt (u64 little-endian nonce) ────────────────────────
//
// Noise nonce framing: 96-bit nonce = 0x00000000 || u64le(counter). The
// fixed vector below uses the RFC 8439 key/AAD/plaintext with
// nonce = 0x0102030405060708 (LE bytes 08 07 06 05 04 03 02 01), expected
// values generated with the OpenSSL oracle (see Task 1 report).

const NOISE_AEAD = {
  key: Uint8Array.from({ length: 32 }, (_, i) => 0x80 + i),
  nonce: 0x0102030405060708n,
  aad: fromHex("50515253c0c1c2c3c4c5c6c7"),
  plaintext: new TextEncoder().encode(
    "Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.",
  ),
  ciphertext:
    "b8d08ad235a97bbc4dd551cc6b1911b3961e31851245f58e7f581dbe0627e811f7465bd39dd94d5546bf48ac1a2dd43cbda6f795d215ec93abb888192adfa41a10ee4bb956f2bbbe7294b33f513dbeeba962d9c2f8cc18c88a9171bebc9ffb116a4383f49408e055ad9ca5fa75dccb65b94f",
  tag: "d963bce6a7bfba85f5b845fd1ee7f5f1",
};

describe("Noise encrypt/decrypt (u64 LE nonce framing)", () => {
  it("encrypts with the Noise nonce framing to the oracle ciphertext", () => {
    const { key, nonce, aad, plaintext, ciphertext, tag } = NOISE_AEAD;
    const sealed = encrypt(key, nonce, aad, plaintext);
    expect(sealed).toHaveLength(plaintext.length + 16);
    expect(toHex(sealed.subarray(0, -16))).toBe(ciphertext);
    expect(toHex(sealed.subarray(-16))).toBe(tag);
  });

  it("decrypts its own ciphertext", () => {
    const { key, nonce, aad, plaintext } = NOISE_AEAD;
    const sealed = encrypt(key, nonce, aad, plaintext);
    const opened = decrypt(key, nonce, aad, sealed);
    expect(opened).not.toBeNull();
    expect(toHex(opened!)).toBe(toHex(plaintext));
  });

  it("builds the same nonce as the raw 12-byte seal path", () => {
    const { key, aad, plaintext } = NOISE_AEAD;
    const nonce12 = new Uint8Array(12);
    new DataView(nonce12.buffer).setBigUint64(4, 0x0001020304050607n, true);
    const viaNoise = encrypt(key, 0x0001020304050607n, aad, plaintext);
    const viaRaw = chacha20Poly1305Seal(key, nonce12, aad, plaintext);
    expect(toHex(viaNoise)).toBe(toHex(viaRaw));
  });

  it("accepts a plain number nonce (u64 range)", () => {
    const { key, aad, plaintext } = NOISE_AEAD;
    const nonce12 = new Uint8Array(12);
    new DataView(nonce12.buffer).setBigUint64(4, 5n, true);
    expect(toHex(encrypt(key, 5, aad, plaintext))).toBe(
      toHex(chacha20Poly1305Seal(key, nonce12, aad, plaintext)),
    );
  });

  it("rejects tampered ciphertext with null", () => {
    const { key, nonce, aad, plaintext } = NOISE_AEAD;
    const sealed = encrypt(key, nonce, aad, plaintext);
    sealed[5] ^= 0xff;
    expect(decrypt(key, nonce, aad, sealed)).toBeNull();
  });

  it("rejects nonces outside the u64 range", () => {
    const { key, aad, plaintext } = NOISE_AEAD;
    expect(() => encrypt(key, -1, aad, plaintext)).toThrow();
    expect(() => encrypt(key, 2n ** 64n, aad, plaintext)).toThrow();
    expect(() => decrypt(key, -1, aad, new Uint8Array(0))).toThrow();
  });
});
