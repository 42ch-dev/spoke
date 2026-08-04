/**
 * libp2p Noise identity payload — `NoiseHandshakePayload` protobuf codec
 * and the domain-separated Ed25519 static-key signature (frozen contract
 * `.mstar/specs/noise-xx-libp2p-contract.md` §4).
 *
 * §4.2 — static-key ↔ long-term-identity binding:
 *
 *     signed_preimage = "noise-libp2p-static-key:" || static_public_x25519
 *     identity_sig     = Ed25519.Sign(identity_private, signed_preimage)
 *
 * The signing key is the peer's long-term libp2p identity (the same
 * Ed25519 key that derives the SPOKE `peer_id`); the signed static key is
 * the ephemeral-per-process X25519 key carried in the Noise `s` tokens.
 *
 * §4.3 — `NoiseHandshakePayload` protobuf (libp2p-noise `payload.proto`):
 *
 *     message NoiseExtensions {
 *         repeated bytes  webtransport_certhashes = 1;
 *         repeated string stream_muxers           = 2;
 *     }
 *     message NoiseHandshakePayload {
 *         bytes identity_key = 1;   // libp2p PublicKey protobuf (Ed25519)
 *         bytes identity_sig = 2;   // signature over the static-key preimage
 *         optional NoiseExtensions extensions = 4;
 *     }
 *
 * Flight 1 (initiator → responder) carries the default message — encoded
 * as zero bytes. WebTransport certhash extensions are parsed but never
 * required on the golden path (contract §4.3).
 *
 * Ed25519 comes from the existing `src/crypto.ts` helpers (WebCrypto when
 * available, `@noble/ed25519` fallback — the same backend matrix as the
 * SPOKE hello); no new dependency. Imports stay inside `src/noise/**` plus
 * the shared src-root helpers already in the default bundle.
 */
import { concatBytes } from "@noble/hashes/utils.js";

import { signEd25519, verifyEd25519 } from "../crypto.js";

/** Domain-separated static-key signing string (contract §4.2, ASCII). */
export const STATIC_KEY_DOMAIN = "noise-libp2p-static-key:";

const STATIC_KEY_DOMAIN_BYTES = new TextEncoder().encode(STATIC_KEY_DOMAIN);

// ── Minimal protobuf wire helpers (fixed small messages only) ──────────────

function encodeVarint(value: number | bigint): Uint8Array {
  let v = BigInt(value);
  const bytes: number[] = [];
  while (v >= 0x80n) {
    bytes.push(Number(v & 0x7fn) | 0x80);
    v >>= 7n;
  }
  bytes.push(Number(v));
  return Uint8Array.from(bytes);
}

function readVarint(
  bytes: Uint8Array,
  offset: number,
): { value: bigint; next: number } {
  let result = 0n;
  let shift = 0n;
  let i = offset;
  // Max 10 bytes for a 64-bit varint; continuation bit beyond that is
  // malformed input, not a larger integer.
  while (i < bytes.length && i - offset < 10) {
    const b = bytes[i++];
    result |= BigInt(b & 0x7f) << shift;
    if ((b & 0x80) === 0) return { value: result, next: i };
    shift += 7n;
  }
  throw new Error("malformed protobuf varint");
}

function encodeVarintField(num: number, value: number | bigint): Uint8Array {
  return concatBytes(encodeVarint((num << 3) | 0), encodeVarint(value));
}

function encodeBytesField(num: number, data: Uint8Array): Uint8Array {
  return concatBytes(encodeVarint((num << 3) | 2), encodeVarint(data.length), data);
}

interface ParsedField {
  num: number;
  wire: number;
  /** wire type 2 payload. */
  bytes?: Uint8Array;
  /** wire type 0 value. */
  varint?: bigint;
}

/**
 * Parse a protobuf message, skipping unknown fields. Wire types 0 (varint),
 * 1 (64-bit), 2 (length-delimited) and 5 (32-bit) are consumed; groups
 * (3/4) are rejected as malformed. Throws on truncated input.
 */
function parseProto(bytes: Uint8Array): ParsedField[] {
  const fields: ParsedField[] = [];
  let offset = 0;
  while (offset < bytes.length) {
    const tag = readVarint(bytes, offset);
    offset = tag.next;
    const num = Number(tag.value >> 3n);
    const wire = Number(tag.value & 0x7n);
    if (num === 0) throw new Error("malformed protobuf: field number 0");
    if (wire === 0) {
      const value = readVarint(bytes, offset);
      offset = value.next;
      fields.push({ num, wire, varint: value.value });
    } else if (wire === 1) {
      if (offset + 8 > bytes.length) throw new Error("truncated protobuf fixed64");
      offset += 8;
    } else if (wire === 2) {
      const length = readVarint(bytes, offset);
      offset = length.next;
      const size = Number(length.value);
      if (offset + size > bytes.length) {
        throw new Error("truncated protobuf length-delimited field");
      }
      fields.push({ num, wire, bytes: bytes.slice(offset, offset + size) });
      offset += size;
    } else if (wire === 5) {
      if (offset + 4 > bytes.length) throw new Error("truncated protobuf fixed32");
      offset += 4;
    } else {
      throw new Error(`malformed protobuf: unsupported wire type ${wire}`);
    }
  }
  return fields;
}

// ── §4.2 static-key signature ───────────────────────────────────────────────

/**
 * Sign a Noise static X25519 public key with the long-term Ed25519
 * identity seed (contract §4.2). Returns the raw 64-byte signature.
 */
export async function signNoiseStaticKey(
  identitySeed: Uint8Array,
  staticPublicX25519: Uint8Array,
): Promise<Uint8Array> {
  return signEd25519(
    identitySeed,
    concatBytes(STATIC_KEY_DOMAIN_BYTES, staticPublicX25519),
  );
}

/**
 * Verify an Ed25519 signature over `STATIC_KEY_DOMAIN || static_public`
 * against an identity public key (contract §4.2). Returns `false` (never
 * throws) for any malformed input — a bad signature is an authentication
 * failure, not a crash.
 */
export async function verifyNoiseStaticKey(
  identityPublic: Uint8Array,
  staticPublicX25519: Uint8Array,
  signature: Uint8Array,
): Promise<boolean> {
  try {
    return await verifyEd25519(
      identityPublic,
      concatBytes(STATIC_KEY_DOMAIN_BYTES, staticPublicX25519),
      signature,
    );
  } catch {
    return false;
  }
}

// ── §4.3 libp2p PublicKey protobuf (identity_key) ───────────────────────────

/**
 * Encode a raw 32-byte Ed25519 public key as the libp2p `PublicKey`
 * protobuf (field 1 varint Type = 1 / Ed25519, field 2 bytes Data) — the
 * 36-byte envelope used in identity multihash / `peer_id` derivation.
 */
export function encodeIdentityPublicKey(ed25519Public: Uint8Array): Uint8Array {
  if (ed25519Public.length !== 32) {
    throw new Error("Ed25519 public key must be 32 bytes");
  }
  return concatBytes(encodeVarintField(1, 1), encodeBytesField(2, ed25519Public));
}

/**
 * Decode the libp2p `PublicKey` protobuf back to the raw 32-byte Ed25519
 * public key. `undefined` for anything that is not an Ed25519 key (other
 * key types, malformed/truncated encodings) — fail closed.
 */
export function decodeIdentityPublicKey(bytes: Uint8Array): Uint8Array | undefined {
  let keyType: bigint | undefined;
  let data: Uint8Array | undefined;
  try {
    for (const field of parseProto(bytes)) {
      if (field.num === 1 && field.wire === 0) keyType = field.varint;
      else if (field.num === 2 && field.wire === 2) data = field.bytes;
    }
  } catch {
    return undefined;
  }
  if (keyType !== 1n || data === undefined || data.length !== 32) return undefined;
  return data;
}

// ── §4.3 NoiseHandshakePayload protobuf ─────────────────────────────────────

/** `NoiseExtensions` message (contract §4.3). */
export interface NoiseExtensions {
  /** WebTransport certificate hashes (field 1) — parsed, never required. */
  webtransportCerthashes?: Uint8Array[];
  /** Preferred stream muxers (field 2). */
  streamMuxers?: string[];
}

/** Decoded `NoiseHandshakePayload` (contract §4.3). */
export interface NoiseHandshakePayload {
  /** libp2p `PublicKey` protobuf bytes (36 bytes for Ed25519). */
  identityKey?: Uint8Array;
  /** Ed25519 signature over `STATIC_KEY_DOMAIN || static_public` (§4.2). */
  identitySig?: Uint8Array;
  /** Optional extensions (field 4) — omitted on the golden path. */
  extensions?: NoiseExtensions;
}

/**
 * Encode a `NoiseHandshakePayload`. The default message (no fields) is the
 * zero-length encoding used for flight 1 (contract §4.3 field table).
 */
export function encodeHandshakePayload(payload: NoiseHandshakePayload): Uint8Array {
  const parts: Uint8Array[] = [];
  if (payload.identityKey !== undefined) {
    parts.push(encodeBytesField(1, payload.identityKey));
  }
  if (payload.identitySig !== undefined) {
    parts.push(encodeBytesField(2, payload.identitySig));
  }
  if (payload.extensions !== undefined) {
    const extParts: Uint8Array[] = [];
    for (const hash of payload.extensions.webtransportCerthashes ?? []) {
      extParts.push(encodeBytesField(1, hash));
    }
    for (const muxer of payload.extensions.streamMuxers ?? []) {
      extParts.push(encodeBytesField(2, new TextEncoder().encode(muxer)));
    }
    parts.push(encodeBytesField(4, concatBytes(...extParts)));
  }
  return concatBytes(...parts);
}

/**
 * Decode a `NoiseHandshakePayload`. Unknown fields (including future
 * extension numbers) are skipped; malformed protobuf throws (a malformed
 * handshake payload is invalid data that aborts the handshake).
 */
export function decodeHandshakePayload(bytes: Uint8Array): NoiseHandshakePayload {
  const out: NoiseHandshakePayload = {};
  for (const field of parseProto(bytes)) {
    if (field.num === 1 && field.wire === 2 && field.bytes !== undefined) {
      out.identityKey = field.bytes;
    } else if (field.num === 2 && field.wire === 2 && field.bytes !== undefined) {
      out.identitySig = field.bytes;
    } else if (field.num === 4 && field.wire === 2 && field.bytes !== undefined) {
      const extensions: NoiseExtensions = {};
      for (const sub of parseProto(field.bytes)) {
        if (sub.num === 1 && sub.wire === 2 && sub.bytes !== undefined) {
          (extensions.webtransportCerthashes ??= []).push(sub.bytes);
        } else if (sub.num === 2 && sub.wire === 2 && sub.bytes !== undefined) {
          (extensions.streamMuxers ??= []).push(
            new TextDecoder().decode(sub.bytes),
          );
        }
      }
      out.extensions = extensions;
    }
  }
  return out;
}
