/**
 * RFC 8785 JCS canonicalization for connect hello signed objects.
 *
 * Normative: `.mstar/specs/spoke-connect.md` §Signature canonicalization;
 * mirrors `crates/spoke-connect/src/core/hello_crypto.rs`
 * `canonical_hello_bytes` (spoke-connect-hello-jcs-v1).
 *
 * The signed object is exactly `{protocol_version, peer_id, nonce, host}` —
 * top-level hello `extensions` and the `signature` field are not covered.
 *
 * Uses the pinned `canonicalize` npm package (RFC 8785). Golden byte parity
 * is pinned in `tests/golden.test.ts` (AD-P0-2 evidence gate): if a
 * dependency update ever diverges from `GOLDEN_JCS_HEX`, port the hand-rolled
 * JCS subset from tooling/connect-identity-proof/proof.mjs into this module
 * (AD-P0-2 fallback) instead of weakening the golden vector.
 *
 * Absent optional manifest members (e.g. `host.authority`) must be omitted,
 * never emitted as `null` — `canonicalize` omits `undefined` members, which
 * matches the Rust `skip_serializing_if` behavior. An explicit `null` would
 * canonicalize to different bytes (and therefore a different signature).
 */

import canonicalize from "canonicalize";
import type { HostCapabilityManifest } from "@42ch/spoke-schemas";

/** Connect protocol version (not data `schema_version`); protocol version 1 is current. */
const PROTOCOL_VERSION = 1;

/**
 * Canonicalize the signed hello object (`{protocol_version, peer_id, nonce,
 * host}`) with RFC 8785 JCS and return the UTF-8 bytes the hello signature
 * covers.
 */
export function canonicalHelloBytes(
  peerId: string,
  nonce: string,
  host: HostCapabilityManifest,
): Uint8Array {
  const jcs = canonicalize({
    protocol_version: PROTOCOL_VERSION,
    peer_id: peerId,
    nonce,
    host,
  });
  if (jcs === undefined) {
    throw new Error(
      "canonicalize returned undefined — signed hello object is not JSON-serializable",
    );
  }
  return new TextEncoder().encode(jcs);
}
