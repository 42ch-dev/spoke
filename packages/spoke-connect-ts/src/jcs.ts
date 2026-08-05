/**
 * RFC 8785 JCS canonicalization for connect hello signed objects.
 *
 * Normative: `.mstar/specs/spoke-connect.md` §Signature canonicalization;
 * mirrors `crates/spoke-connect/src/core/hello_crypto.rs`
 * `canonical_hello_bytes` (spoke-connect-hello-jcs-v1).
 *
 * The signed object is exactly `{protocol_version, peer_id, nonce, host}`
 * for the **initiator** hello, and exactly
 * `{protocol_version, peer_id, nonce, host, peer_nonce}` for the
 * **responder** hello (`peer_nonce` = the initiator's nonce — dial
 * binding). Top-level hello `extensions` and the `signature` field are not
 * covered.
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
 * The optional `peer_nonce` follows the same rule: when `undefined` (the
 * initiator role) the member is absent entirely, so the 4-field canonical
 * bytes stay byte-identical to the pre-dial-binding wire.
 */

import canonicalize from "canonicalize";
import type { HostCapabilityManifest } from "@42ch/spoke-schemas";

import { PROTOCOL_VERSION } from "./core/version.js";

/**
 * Canonicalize the signed hello object with RFC 8785 JCS and return the
 * UTF-8 bytes the hello signature covers: the 4-field initiator object when
 * `peerNonce` is `undefined`, the 5-field responder object (dial binding)
 * otherwise.
 */
export function canonicalHelloBytes(
  peerId: string,
  nonce: string,
  host: HostCapabilityManifest,
  peerNonce?: string,
): Uint8Array {
  const jcs = canonicalize({
    protocol_version: PROTOCOL_VERSION,
    peer_id: peerId,
    nonce,
    ...(peerNonce !== undefined ? { peer_nonce: peerNonce } : {}),
    host,
  });
  if (jcs === undefined) {
    throw new Error(
      "canonicalize returned undefined — signed hello object is not JSON-serializable",
    );
  }
  return new TextEncoder().encode(jcs);
}
