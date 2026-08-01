/**
 * Ed25519 sign/verify and key encoding for SPOKE connect hello (raw key
 * bytes, no libp2p Keypair).
 *
 * Ported from tooling/connect-identity-proof/proof.mjs; normative rules
 * `.mstar/specs/spoke-connect.md` §Signature canonicalization / §Identity
 * binding (mirrors `crates/spoke-connect/src/core/hello_crypto.rs`).
 *
 * - Raw 32-byte seed / public key at the boundary.
 * - PKCS8 / SPKI DER wrapping (RFC 8410) for WebCrypto import.
 * - Signatures are raw 64 bytes, encoded base64url without padding.
 * - Backend matrix: WebCrypto `Ed25519` first when `crypto.subtle` accepts
 *   it (Node ≥ 22, modern browsers), else `@noble/ed25519`. Both paths must
 *   satisfy the golden sign/verify vectors.
 */

import * as nobleEd25519 from "@noble/ed25519";
import { sha512 } from "@noble/hashes/sha2.js";

// noble-ed25519 v3 keeps its SHA-512 provider slots caller-owned (the package
// is dependency-free by default); sync sign/verify/getPublicKey require the
// wired provider before first use.
nobleEd25519.hashes.sha512 = sha512;

function subtle(): SubtleCrypto | undefined {
  return globalThis.crypto?.subtle;
}

/**
 * WebCrypto's `BufferSource` requires `ArrayBuffer`-backed views (TS 5.7+
 * typed-array generics). Every buffer this module produces or receives is
 * ArrayBuffer-backed in practice (`Uint8Array` constructors/`from`, noble
 * outputs, DER wrappers); normalize the type at the boundary instead of
 * widening the public API or copying bytes.
 */
function asBufferSource(bytes: Uint8Array): BufferSource {
  return bytes as unknown as BufferSource;
}

// ── base64url (no padding) ────────────────────────────────────────────────

/** Encode bytes as base64url without padding. */
export function base64UrlEncode(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  const b64 = btoa(bin);
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

/** Decode base64url (padding optional) into bytes. */
export function base64UrlDecode(s: string): Uint8Array {
  const pad = s.length % 4 === 0 ? "" : "=".repeat(4 - (s.length % 4));
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/") + pad;
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

// ── DER wrapping (RFC 8410) ───────────────────────────────────────────────

/** PKCS8 DER wrapping a raw 32-byte Ed25519 seed (RFC 8410). */
export function ed25519SeedToPkcs8(seed: Uint8Array): Uint8Array {
  if (seed.length !== 32) throw new Error("seed must be 32 bytes");
  // 30 2e 02 01 00 30 05 06 03 2b 65 70 04 22 04 20 || seed
  const header = Uint8Array.of(
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70,
    0x04, 0x22, 0x04, 0x20,
  );
  const out = new Uint8Array(header.length + seed.length);
  out.set(header, 0);
  out.set(seed, header.length);
  return out;
}

/** SPKI DER wrapping a raw 32-byte Ed25519 public key (RFC 8410). */
export function ed25519PubkeyToSpki(pubkey: Uint8Array): Uint8Array {
  if (pubkey.length !== 32) throw new Error("pubkey must be 32 bytes");
  // 30 2a 30 05 06 03 2b 65 70 03 21 00 || pubkey
  const header = Uint8Array.of(
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
  );
  const out = new Uint8Array(header.length + pubkey.length);
  out.set(header, 0);
  out.set(pubkey, header.length);
  return out;
}

// ── backend selection ─────────────────────────────────────────────────────

let probe: Promise<boolean> | null = null;

/**
 * Whether this runtime's WebCrypto accepts Ed25519 keys (Node ≥ 22, modern
 * browsers). Node 20 CI does not — the `@noble` fallback covers it.
 */
export function webcryptoEd25519Available(): Promise<boolean> {
  probe ??= (async () => {
    const s = subtle();
    if (!s) return false;
    try {
      await s.importKey(
        "pkcs8",
        asBufferSource(ed25519SeedToPkcs8(new Uint8Array(32))),
        { name: "Ed25519" },
        false,
        ["sign"],
      );
      return true;
    } catch {
      return false;
    }
  })();
  return probe;
}

// ── key derivation ────────────────────────────────────────────────────────

/**
 * Derive the 32-byte Ed25519 public key for a 32-byte seed.
 *
 * `@noble/ed25519` is the sole seed→public-key operation: WebCrypto has no
 * Ed25519 public-key export from a private key (Node rejects
 * `exportKey("spki")` on Ed25519 private keys), so there is no WebCrypto
 * path to implement. Both sign/verify backends consume the resulting bytes.
 */
export function getPublicKeyEd25519(seed: Uint8Array): Uint8Array {
  if (seed.length !== 32) throw new Error("seed must be 32 bytes");
  return nobleEd25519.getPublicKey(seed);
}

// ── sign / verify ─────────────────────────────────────────────────────────

/**
 * Sign `message` with a raw 32-byte Ed25519 seed. Returns the raw 64-byte
 * signature. Backend: WebCrypto when available, else `@noble/ed25519`.
 */
export async function signEd25519(
  seed: Uint8Array,
  message: Uint8Array,
): Promise<Uint8Array> {
  if (await webcryptoEd25519Available()) {
    const s = subtle()!;
    const key = await s.importKey(
      "pkcs8",
      asBufferSource(ed25519SeedToPkcs8(seed)),
      { name: "Ed25519" },
      false,
      ["sign"],
    );
    const sig = await s.sign(
      { name: "Ed25519" },
      key,
      asBufferSource(message),
    );
    return new Uint8Array(sig);
  }
  return nobleEd25519.sign(message, seed);
}

/**
 * Verify a raw 64-byte signature over `message` with a 32-byte Ed25519
 * public key. Backend: WebCrypto when available, else `@noble/ed25519`.
 */
export async function verifyEd25519(
  pubkey: Uint8Array,
  message: Uint8Array,
  signature: Uint8Array,
): Promise<boolean> {
  if (await webcryptoEd25519Available()) {
    const s = subtle()!;
    const key = await s.importKey(
      "spki",
      asBufferSource(ed25519PubkeyToSpki(pubkey)),
      { name: "Ed25519" },
      false,
      ["verify"],
    );
    return s.verify(
      { name: "Ed25519" },
      key,
      asBufferSource(signature),
      asBufferSource(message),
    );
  }
  return nobleEd25519.verify(signature, message, pubkey);
}

/**
 * `@noble/ed25519` sign — forces the fallback path (sync). Tests exercise
 * both backends explicitly so golden parity holds on every runtime.
 */
export function signEd25519Noble(
  seed: Uint8Array,
  message: Uint8Array,
): Uint8Array {
  if (seed.length !== 32) throw new Error("seed must be 32 bytes");
  return nobleEd25519.sign(message, seed);
}

/**
 * `@noble/ed25519` verify — forces the fallback path (sync).
 */
export function verifyEd25519Noble(
  pubkey: Uint8Array,
  message: Uint8Array,
  signature: Uint8Array,
): boolean {
  return nobleEd25519.verify(signature, message, pubkey);
}
