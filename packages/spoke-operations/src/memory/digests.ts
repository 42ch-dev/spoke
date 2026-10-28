/**
 * JCS / SHA-256 digest helpers (§6.7 `content_digest`, §6.10
 * `payload_digest`, §6.11 bundle `digest`, §6.13 `chunk_digest`):
 * RFC 8785 canonicalization → SHA-256 → 64 lowercase hex.
 *
 * Canonicalization uses the `canonicalize` package — the reference RFC 8785
 * implementation, also used by spoke-connect-ts capability tokens, so TS
 * and Rust (`serde_jcs`) digests agree (cross-pinned known vectors in the
 * tests mirror the Rust pins exactly).
 */

import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex, utf8ToBytes } from "@noble/hashes/utils.js";
import canonicalize from "canonicalize";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";

/**
 * SHA-256 over the RFC 8785 JCS canonicalization of `value`, as 64
 * lowercase hex. Rejects with `INVALID_INPUT` when the value cannot be
 * canonicalized (circular references, BigInt, non-finite numbers — RFC 8785
 * has no representation for the latter).
 */
export function jcsSha256Hex(value: unknown): SpokeResult<string> {
  let canonical: string | undefined;
  try {
    canonical = canonicalize(value);
  } catch {
    canonical = undefined;
  }
  if (canonical === undefined) {
    return spokeReject(
      "INVALID_INPUT",
      "JCS canonicalization failed: value is not JSON-canonicalizable (RFC 8785)",
    );
  }
  return spokeOk(bytesToHex(sha256(utf8ToBytes(canonical))));
}

/**
 * Digest over `value` with `excluded_field` removed — §6.13 `chunk_digest`
 * (chunk object minus `chunk_digest`) and §6.11 bundle `digest` (bundle
 * object minus `digest`).
 */
export function jcsSha256HexExcluding(
  value: unknown,
  excludedField: string,
): SpokeResult<string> {
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    const { [excludedField]: _removed, ...rest } = value as Record<string, unknown>;
    return jcsSha256Hex(rest);
  }
  return jcsSha256Hex(value);
}
