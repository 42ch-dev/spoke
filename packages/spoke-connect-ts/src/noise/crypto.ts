/**
 * Noise crypto primitives — thin wrappers over the noble suite with
 * Noise-constant semantics (frozen contract
 * `.mstar/specs/noise-xx-libp2p-contract.md` §7).
 *
 * Surface (consumed by `src/noise/xx.ts`, Task 2):
 *
 *   - `dh(priv, pub)`                    X25519 shared secret (RFC 7748)
 *   - `getPublicKey(priv)`               X25519 public key (RFC 7748)
 *   - `hkdf(chainingKey, ikm)`           Noise HKDF → { chainingKey, key }
 *                                         (Noise spec §5.2, SHA-256)
 *   - `encrypt(key, nonce, aad, pt)`     ChaCha20-Poly1305 seal, Noise
 *                                         nonce = 4 zero bytes || u64le
 *   - `decrypt(key, nonce, aad, ct)`     ChaCha20-Poly1305 open; null on
 *                                         auth failure (Noise nonce framing)
 *
 * Plus the raw RFC-shaped helpers the wrappers build on (`hkdfSha256`,
 * `chacha20Poly1305Seal`/`chacha20Poly1305Open` with an explicit 12-byte
 * nonce), which carry the RFC 5869 / RFC 8439 vector gates.
 *
 * Imports stay inside `src/noise/**` only (default-bundle isolation rule,
 * plan "Global Constraints"). No WebCrypto: noble is the portable source
 * per architect decision.
 */
import { chacha20poly1305 } from "@noble/ciphers/chacha.js";
import { x25519 } from "@noble/curves/ed25519.js";
import { hkdf as nobleHkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";

// ── X25519 DH (RFC 7748) ──────────────────────────────────────────────────

/**
 * X25519 public key for a 32-byte private key (RFC 7748 §5).
 */
export function getPublicKey(privateKey: Uint8Array): Uint8Array {
  return x25519.getPublicKey(privateKey);
}

/**
 * X25519 Diffie-Hellman: 32-byte shared secret from a 32-byte private key
 * and a 32-byte public key (Noise `DH` operation).
 */
export function dh(privateKey: Uint8Array, publicKey: Uint8Array): Uint8Array {
  return x25519.getSharedSecret(privateKey, publicKey);
}

// ── HKDF-SHA256 (RFC 5869) ────────────────────────────────────────────────

/**
 * General HKDF-SHA256 (RFC 5869): `HKDF-Extract(salt, ikm)` then
 * `HKDF-Expand(prk, info, length)`. Argument order follows the RFC
 * function signature `HKDF(salt, IKM, info, L)`.
 */
export function hkdfSha256(
  salt: Uint8Array,
  ikm: Uint8Array,
  info: Uint8Array,
  length: number,
): Uint8Array {
  return nobleHkdf(sha256, ikm, salt, info, length);
}

/**
 * Noise HKDF (Noise spec §5.2, hash = SHA-256) with the
 * chaining-key / input-key-material split: returns BOTH 32-byte outputs —
 * the new chaining key and the cipher key — from
 * `HKDF(chainingKey, ikm)` with empty info.
 */
export function hkdf(
  chainingKey: Uint8Array,
  inputKeyMaterial: Uint8Array,
): { chainingKey: Uint8Array; key: Uint8Array } {
  const okm = hkdfSha256(chainingKey, inputKeyMaterial, new Uint8Array(0), 64);
  return {
    chainingKey: okm.subarray(0, 32),
    key: okm.subarray(32, 64),
  };
}

// ── ChaCha20-Poly1305 (RFC 8439) ──────────────────────────────────────────

/**
 * Raw ChaCha20-Poly1305 AEAD seal with an explicit 96-bit nonce
 * (RFC 8439 §2.8). Returns `ciphertext || 16-byte tag`.
 */
export function chacha20Poly1305Seal(
  key: Uint8Array,
  nonce12: Uint8Array,
  aad: Uint8Array,
  plaintext: Uint8Array,
): Uint8Array {
  return chacha20poly1305(key, nonce12, aad).encrypt(plaintext);
}

/**
 * Raw ChaCha20-Poly1305 AEAD open with an explicit 96-bit nonce.
 * Returns the plaintext, or `null` when authentication fails (noble
 * throws; the wrapper converts to a null result for the state machine).
 */
export function chacha20Poly1305Open(
  key: Uint8Array,
  nonce12: Uint8Array,
  aad: Uint8Array,
  ciphertext: Uint8Array,
): Uint8Array | null {
  try {
    return chacha20poly1305(key, nonce12, aad).decrypt(ciphertext);
  } catch {
    return null;
  }
}

/**
 * Encode a Noise nonce (64-bit counter) into the 96-bit ChaCha nonce:
 * `0x00000000 || u64le(nonce)` (Noise spec; frozen contract §3).
 */
function noiseNonce12(nonce: number | bigint): Uint8Array {
  const n = BigInt(nonce);
  if (n < 0n || n >= 1n << 64n) {
    throw new Error(`noise nonce out of u64 range: ${nonce}`);
  }
  const nonce12 = new Uint8Array(12);
  new DataView(nonce12.buffer).setBigUint64(4, n, true);
  return nonce12;
}

/**
 * Noise ChaCha20-Poly1305 seal. `nonce` is the Noise 64-bit little-endian
 * counter (must be reused with a given key); returns
 * `ciphertext || 16-byte tag`.
 */
export function encrypt(
  key: Uint8Array,
  nonce: number | bigint,
  aad: Uint8Array,
  plaintext: Uint8Array,
): Uint8Array {
  return chacha20Poly1305Seal(key, noiseNonce12(nonce), aad, plaintext);
}

/**
 * Noise ChaCha20-Poly1305 open. Returns the plaintext, or `null` when the
 * tag does not verify (tampered ciphertext / AAD / wrong key / wrong
 * nonce).
 */
export function decrypt(
  key: Uint8Array,
  nonce: number | bigint,
  aad: Uint8Array,
  ciphertext: Uint8Array,
): Uint8Array | null {
  return chacha20Poly1305Open(key, noiseNonce12(nonce), aad, ciphertext);
}
