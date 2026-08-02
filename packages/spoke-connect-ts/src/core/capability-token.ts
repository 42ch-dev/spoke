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

/**
 * Wire keys of the token wrapper — the runtime whitelist SSOT used by both
 * the proof-shape guard and the canonical wrapper construction (mirrors the
 * Rust struct fields; see `CapabilityTokenProof`).
 */
const PROOF_KEYS: readonly string[] = ["v", "claims", "sig"];

/**
 * Wire keys of the signed claims object — the runtime whitelist SSOT shared
 * by `assertProofShape` (reject unknown keys) and the canonical JCS
 * projection (pick only the normative keys), mirroring the Rust struct
 * fields (`serde(deny_unknown_fields)`). Keep in sync with
 * `CapabilityClaims`.
 */
const CLAIM_KEYS: readonly string[] = [
  "iss",
  "sub",
  "aud",
  "capabilities",
  "exp",
  "iat",
  "jti",
];

/** Smallest double above any Rust `u64` (`2^64`); `n < 2^64` ⇔ `0 ≤ n ≤ 2^64 − 1` for doubles. */
const U64_EXCLUSIVE_LIMIT = 2 ** 64;

/** A JSON integer Rust accepts in a `u64` position: finite, integral, `0..2^64 − 1`. */
function isU64JsonInteger(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    Number.isInteger(value) &&
    value >= 0 &&
    value < U64_EXCLUSIVE_LIMIT
  );
}

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
 * Project `claims` onto the normative key set ([`CLAIM_KEYS`]), dropping any
 * unknown keys. Both the signed JCS bytes and the wrapper returned by
 * [`issueCapabilityToken`] come from this projection, so the wrapper always
 * equals the signed object (a self-issued token round-trips even when the
 * caller passed extra keys).
 */
function canonicalClaimsObject(claims: CapabilityClaims): CapabilityClaims {
  const source = claims as unknown as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const key of CLAIM_KEYS) {
    const value = source[key];
    if (value !== undefined) out[key] = value;
  }
  return out as unknown as CapabilityClaims;
}

/**
 * JCS-canonicalize the claims object to the UTF-8 bytes the token signature
 * covers. Absent optional claims (`iat` / `jti`) are **omitted**, never
 * emitted as `null` — matches Rust `skip_serializing_if = "Option::is_none"`.
 *
 * Any `canonicalize` failure (non-finite numbers, BigInt, circular
 * references) surfaces as a typed `CoreError("jcs")`, never a raw error.
 */
function canonicalClaimsBytes(claims: CapabilityClaims): Uint8Array {
  let jcs: string | undefined;
  try {
    jcs = canonicalize(canonicalClaimsObject(claims));
  } catch (err) {
    throw new CoreError(
      "jcs",
      `claims object is not JSON-canonicalizable: ${(err as Error).message}`,
    );
  }
  if (jcs === undefined) {
    throw new CoreError("jcs", "claims object is not JSON-serializable");
  }
  return new TextEncoder().encode(jcs);
}

/**
 * Fail-closed runtime claim guard for **issuance**: before any crypto runs,
 * reject claims the verifier would deterministically reject, so
 * `issueCapabilityToken` never signs a token that cannot verify.
 *
 * Mirrors the claim checks in [`assertProofShape`] (required fields, u64
 * JSON-integer rules for `exp` / `iat` via [`isU64JsonInteger`]) with two
 * fail-fast additions: `capabilities` must be **non-empty** (an empty grant
 * authorizes nothing) and `jti`, when present, must be **non-empty** (verify
 * rule 8 rejects empty `jti`). Unknown keys are **not** rejected here —
 * [`canonicalClaimsObject`] drops them before signing, mirroring the Rust
 * typed struct that cannot even carry extra keys.
 */
function assertClaimsShape(claims: unknown): asserts claims is CapabilityClaims {
  const malformed = (what: string): never => {
    throw new CoreError("token_invalid", `claims are malformed: ${what}`);
  };
  if (typeof claims !== "object" || claims === null) {
    malformed("claims must be an object");
  }
  const c = claims as Record<string, unknown>;
  if (typeof c.iss !== "string") {
    malformed("iss must be a string");
  }
  if (typeof c.sub !== "string") {
    malformed("sub must be a string");
  }
  if (typeof c.aud !== "string") {
    malformed("aud must be a string");
  }
  if (
    !Array.isArray(c.capabilities) ||
    c.capabilities.length === 0 ||
    !c.capabilities.every((item) => typeof item === "string")
  ) {
    malformed("capabilities must be a non-empty string array");
  }
  // Rust types exp / iat as u64; mirror the verifier's u64 rules exactly so
  // issuance cannot sign a token the verifier would reject on shape.
  if (!isU64JsonInteger(c.exp)) {
    malformed("exp must be a u64 JSON integer");
  }
  if (c.iat !== undefined && !isU64JsonInteger(c.iat)) {
    malformed("iat must be a u64 JSON integer when present");
  }
  if (c.jti !== undefined && (typeof c.jti !== "string" || c.jti === "")) {
    malformed("jti must be a non-empty string when present");
  }
}

/**
 * Issue a capability token: canonicalize `claims` with JCS, sign with the
 * issuer Ed25519 secret key (32-byte seed), and wrap the result.
 *
 * Claims are shape-validated before signing ([`assertClaimsShape`]) — a
 * token that could never verify is rejected at issuance with
 * `CoreError("token_invalid")` instead of being signed.
 *
 * The issuer's derived `peer_id` MUST equal `claims.iss` — the token must
 * be issued by the authority it names, or it cannot verify.
 */
export async function issueCapabilityToken(
  issuerSecret: Uint8Array,
  claims: CapabilityClaims,
): Promise<CapabilityTokenProof> {
  assertClaimsShape(claims);
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
  // Return the whitelisted projection that was actually signed — not the
  // caller's object — so the wrapper claims always equal the signed bytes
  // (extra keys carried by `claims` at runtime are dropped, mirroring the
  // Rust typed struct).
  return {
    v: TOKEN_VERSION,
    claims: canonicalClaimsObject(claims),
    sig: signature,
  };
}

/**
 * Fail-closed runtime shape guard for wire-parsed proofs.
 *
 * The TS public types do not enforce shapes at runtime, so a proof parsed
 * from the wire (`JSON.parse` of an OpaqueJson) can violate the
 * `CapabilityTokenProof` shape. This guard rejects any missing / malformed /
 * unknown field with `CoreError("token_invalid")` **before** the version
 * check and any crypto runs — a downstream TypeError (e.g. spreading a
 * missing `capabilities`) must never escape as a non-CoreError. Mirrors the
 * Rust `serde(deny_unknown_fields)` deserialization, which rejects
 * malformed proofs before `verify_capability_token` runs.
 */
function assertProofShape(value: unknown): asserts value is CapabilityTokenProof {
  const malformed = (what: string): never => {
    throw new CoreError("token_invalid", `proof is malformed: ${what}`);
  };
  if (typeof value !== "object" || value === null) {
    malformed("proof must be an object with v, claims, sig");
  }
  const proof = value as Record<string, unknown>;
  const wrapperKeys = Object.keys(proof);
  for (const key of wrapperKeys) {
    if (!PROOF_KEYS.includes(key)) {
      malformed(`unknown wrapper key: ${key}`);
    }
  }
  if (!isU64JsonInteger(proof.v)) {
    malformed("v must be a u64 JSON integer");
  }
  if (typeof proof.sig !== "string") {
    malformed("sig must be a string");
  }
  const claims = proof.claims;
  if (typeof claims !== "object" || claims === null) {
    malformed("claims must be an object");
  }
  const c = claims as Record<string, unknown>;
  const claimKeys = Object.keys(c);
  for (const key of claimKeys) {
    if (!CLAIM_KEYS.includes(key)) {
      malformed(`unknown claim key: ${key}`);
    }
  }
  if (typeof c.iss !== "string") {
    malformed("claims.iss must be a string");
  }
  if (typeof c.sub !== "string") {
    malformed("claims.sub must be a string");
  }
  if (typeof c.aud !== "string") {
    malformed("claims.aud must be a string");
  }
  if (
    !Array.isArray(c.capabilities) ||
    !c.capabilities.every((item) => typeof item === "string")
  ) {
    malformed("claims.capabilities must be a string array");
  }
  // Rust types exp / iat / v as u64: `serde_json` rejects non-finite,
  // fractional, negative, and out-of-range values at deserialization.
  // `typeof === "number"` alone lets `Infinity`/fractions through, so the
  // guard must mirror the u64 rules exactly.
  if (!isU64JsonInteger(c.exp)) {
    malformed("claims.exp must be a u64 JSON integer");
  }
  if (c.iat !== undefined && !isU64JsonInteger(c.iat)) {
    malformed("claims.iat must be a u64 JSON integer when present");
  }
  if (c.jti !== undefined && typeof c.jti !== "string") {
    malformed("claims.jti must be a string when present");
  }
}

/**
 * Validate a capability-token proof against this node's trust configuration
 * and the authenticated session peer.
 *
 * Checks, in order (normative §Trust root and validation rules):
 * 0. **Shape guard** — the proof must be `{v, claims, sig}` with the
 *    normative claim set (`iss`, `sub`, `aud`, `capabilities`, `exp`,
 *    optional `iat` / `jti`); missing / malformed / unknown fields reject
 *    with `CoreError("token_invalid")` before any crypto or trust check
 *    (mirrors Rust `serde(deny_unknown_fields)` deserialization).
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
 * Returns the validated grant (`claims.capabilities`) for the dispatch
 * gate — the token does **not** replace the session's
 * `negotiated_capabilities`.
 */
export async function verifyCapabilityToken(
  proof: CapabilityTokenProof,
  trustedIssuers: readonly string[],
  thisPeerId: string,
  sessionPeerId: string,
  now: number,
): Promise<string[]> {
  assertProofShape(proof);
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
  // The spec mandates base64url **without padding**, and Rust decodes with
  // `URL_SAFE_NO_PAD`, which rejects padded input and alternate encodings
  // of the final character's slack bits. A canonical round-trip
  // (`encode(decode(sig)) === sig`) admits exactly the one encoding Rust
  // accepts, so `sig` stays a canonical identifier (replay caches / dedup
  // keys on it).
  if (base64UrlEncode(signature) !== proof.sig) {
    throw new CoreError(
      "token_invalid",
      "signature is not canonical base64url (no padding)",
    );
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
