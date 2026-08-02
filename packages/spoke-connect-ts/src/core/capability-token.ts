/**
 * Capability-token issuance and validation over **raw** Ed25519 key bytes.
 *
 * Ported from `crates/spoke-connect/src/core/capability_token.rs`; normative
 * rules `.mstar/specs/spoke-connect.md` §Method — capability-token:
 * - Signed claims object = exactly `{iss, sub, aud, capabilities, exp}`
 *   plus optional `iat` / `jti`. Unknown claim keys reject (fail closed) so
 *   the JCS bytes stay intentional.
 * - Canonicalize with RFC 8785 JCS (`src/jcs.ts` `canonicalize`) → UTF-8
 *   bytes; sign with the **issuer** Ed25519 private key; the raw 64-byte
 *   signature is encoded base64url without padding. The signature covers
 *   **only** JCS(`claims`) — not the `{v, claims, sig}` wire wrapper.
 * - The issuer key MUST derive `claims.iss`; verification recovers the
 *   issuer public key from the `iss` peer id itself (Ed25519 peer ids use
 *   the identity multihash, so the mapping is an encoding inversion — see
 *   `identity.ts::ed25519PubkeyFromPeerId`).
 *
 * Validation is **offline**: signature + trusted-issuer list + subject /
 * audience / expiry + capability membership. No revocation list, no refresh
 * token, no issuance endpoint (see spec non-goals).
 *
 * Sync on Rust, async on TS — the Ed25519 helpers are async (WebCrypto
 * primary, `@noble/ed25519` fallback, same bytes; see `src/crypto.ts`).
 */

import canonicalize from "canonicalize";

import {
  base64UrlDecode,
  base64UrlEncode,
  getPublicKeyEd25519,
  signEd25519,
  verifyEd25519,
} from "../crypto.js";
import {
  derivePeerIdFromEd25519Pubkey,
  ed25519PubkeyFromPeerId,
} from "../identity.js";
import { CoreError } from "./error.js";

/** Token format version carried in the wire wrapper `v` (protocol version 1 uses `1`). */
export const TOKEN_VERSION = 1;

/** Clock-skew allowance (seconds) applied to `iat`: a token whose `iat` is up to 60s in the future is accepted. */
export const CLOCK_SKEW_SECONDS = 60;

/** The signed claims object of a capability token (normative claim set). */
export interface CapabilityClaims {
  /** Issuer `peer_id` — the string form of the signing key's derived id. */
  iss: string;
  /** Subject `peer_id` — who may present the token. */
  sub: string;
  /** Audience — the verifying node's `peer_id`. */
  aud: string;
  /** Capability names granted to `sub` (e.g. `spoke-baseline`). */
  capabilities: string[];
  /** Expiry as Unix time seconds (UTC); reject when `now >= exp`. */
  exp: number;
  /** Issued-at Unix seconds; when present, reject when `iat` is beyond the clock-skew window ahead of `now`. */
  iat?: number;
  /** Unique token id; when present, must be non-empty. Reserved for a future revocation design — not consulted by protocol version 1 validation. */
  jti?: string;
}

/** The wire `proof` wrapper for the capability-token method. `sig` covers **only** JCS(`claims`). */
export interface CapabilityTokenProof {
  /** Token format version ([`TOKEN_VERSION`]). */
  v: number;
  /** The signed claims object. */
  claims: CapabilityClaims;
  /** base64url (no padding) of the 64 raw Ed25519 signature bytes over JCS(`claims`). */
  sig: string;
}

/**
 * JCS-canonicalize the claims object to the UTF-8 bytes the token signature
 * covers. Absent optional claims (`iat` / `jti`) are **omitted**, never
 * emitted as `null` — matches Rust `skip_serializing_if = "Option::is_none"`.
 */
function canonicalClaimsBytes(claims: CapabilityClaims): Uint8Array {
  const jcs = canonicalize({
    iss: claims.iss,
    sub: claims.sub,
    aud: claims.aud,
    capabilities: claims.capabilities,
    exp: claims.exp,
    ...(claims.iat !== undefined ? { iat: claims.iat } : {}),
    ...(claims.jti !== undefined ? { jti: claims.jti } : {}),
  });
  if (jcs === undefined) {
    throw new CoreError("jcs", "claims object is not JSON-serializable");
  }
  return new TextEncoder().encode(jcs);
}

/**
 * Issue a capability token: canonicalize `claims` with JCS, sign with the
 * issuer Ed25519 secret key (32-byte seed), and wrap the result.
 *
 * The issuer's derived `peer_id` MUST equal `claims.iss` — the token must
 * be issued by the authority it names, or it cannot verify.
 */
export async function issueCapabilityToken(
  issuerSecret: Uint8Array,
  claims: CapabilityClaims,
): Promise<CapabilityTokenProof> {
  const derivedIssuer = derivePeerIdFromEd25519Pubkey(
    getPublicKeyEd25519(issuerSecret),
  );
  if (derivedIssuer !== claims.iss) {
    throw new CoreError(
      "token_invalid",
      `issuer key derives peer id ${derivedIssuer}, not claims.iss ${claims.iss}`,
    );
  }
  const bytes = canonicalClaimsBytes(claims);
  const signature = base64UrlEncode(await signEd25519(issuerSecret, bytes));
  return { v: TOKEN_VERSION, claims, sig: signature };
}

/**
 * Validate a capability-token proof against this node's trust configuration
 * and the authenticated session peer.
 *
 * Checks, in order (normative §Trust root and validation rules):
 * 1. `v` is the current token version.
 * 2. The signature verifies over JCS(`claims`) with the public key that
 *    derives `claims.iss` (recovered from the issuer peer id).
 * 3. `claims.iss` is an exact-string member of `trustedIssuers` (empty
 *    list ⇒ method disabled: every proof is rejected).
 * 4. `claims.sub` equals `sessionPeerId` — the peer that passed the
 *    `noise-peerid` hello. Tokens are not transferable across peers.
 * 5. `claims.aud` equals `thisPeerId` — the verifying node.
 * 6. `exp` is required; reject when `now >= exp`.
 * 7. `iat`, when present, must not be beyond the ±[`CLOCK_SKEW_SECONDS`]
 *    window ahead of `now`.
 * 8. `jti`, when present, must be non-empty.
 *
 * Unknown claims / wrapper keys and malformed shapes are rejected before
 * this function runs (the caller deserializes the opaque proof with a
 * fail-closed shape check). Returns the validated grant
 * (`claims.capabilities`) for the dispatch gate — the token does **not**
 * replace the session's `negotiated_capabilities`.
 */
export async function verifyCapabilityToken(
  proof: CapabilityTokenProof,
  trustedIssuers: readonly string[],
  thisPeerId: string,
  sessionPeerId: string,
  now: number,
): Promise<string[]> {
  if (proof.v !== TOKEN_VERSION) {
    throw new CoreError(
      "token_invalid",
      `unsupported token version ${proof.v} (expected ${TOKEN_VERSION})`,
    );
  }

  // The issuer public key is recovered from the issuer peer id: Ed25519
  // peer ids are identity multihashes, so the string carries the key.
  const issuerPubkey = ed25519PubkeyFromPeerId(proof.claims.iss);
  if (issuerPubkey === undefined) {
    throw new CoreError(
      "token_invalid",
      `iss is not an Ed25519 peer id: ${proof.claims.iss}`,
    );
  }
  const bytes = canonicalClaimsBytes(proof.claims);
  let signature: Uint8Array;
  try {
    signature = base64UrlDecode(proof.sig);
  } catch {
    throw new CoreError("token_invalid", "signature is not valid base64url");
  }
  if (signature.length !== 64) {
    throw new CoreError("token_invalid", "signature is not 64 bytes");
  }
  if (!(await verifyEd25519(issuerPubkey, bytes, signature))) {
    throw new CoreError("token_invalid", "signature does not verify");
  }

  if (!trustedIssuers.includes(proof.claims.iss)) {
    throw new CoreError(
      "token_invalid",
      `issuer ${proof.claims.iss} is not trusted`,
    );
  }
  if (proof.claims.sub !== sessionPeerId) {
    throw new CoreError(
      "token_invalid",
      `subject ${proof.claims.sub} does not match the session peer ${sessionPeerId}`,
    );
  }
  if (proof.claims.aud !== thisPeerId) {
    throw new CoreError(
      "token_invalid",
      `audience ${proof.claims.aud} does not match this node ${thisPeerId}`,
    );
  }
  if (now >= proof.claims.exp) {
    throw new CoreError(
      "token_invalid",
      `token expired at ${proof.claims.exp} (now ${now})`,
    );
  }
  if (
    proof.claims.iat !== undefined &&
    proof.claims.iat > now + CLOCK_SKEW_SECONDS
  ) {
    throw new CoreError(
      "token_invalid",
      `token issued at ${proof.claims.iat} is beyond the ${CLOCK_SKEW_SECONDS}s clock-skew window ahead of now ${now}`,
    );
  }
  if (proof.claims.jti !== undefined && proof.claims.jti === "") {
    throw new CoreError("token_invalid", "jti must be non-empty");
  }
  return [...proof.claims.capabilities];
}
