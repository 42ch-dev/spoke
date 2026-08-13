/**
 * Hello signing and verification over **raw** Ed25519 key bytes
 * (`spoke-connect-hello-jcs-v1`).
 *
 * Ported from `crates/spoke-connect/src/core/hello_crypto.rs`; normative
 * rules `.mstar/specs/spoke-connect.md` §Signature canonicalization /
 * §Identity binding:
 * - Signed object = exactly `{protocol_version, peer_id, nonce, host}` for
 *   the **initiator** hello (4 fields), and exactly
 *   `{protocol_version, peer_id, nonce, host, peer_nonce}` for the
 *   **responder** hello (5 fields, `peer_nonce` = the initiator's nonce —
 *   dial binding). Role is carried by the hello itself: `peer_nonce` is
 *   present in responder hellos, absent in initiator hellos.
 * - Canonicalize with RFC 8785 JCS → UTF-8 bytes (`src/jcs.ts`).
 * - Sign with the Ed25519 private key whose public key derives `peer_id`;
 *   the raw 64-byte signature is encoded base64url without padding.
 * - Top-level hello `extensions` and the `signature` field are **not** part
 *   of the signed object.
 *
 * Wire-shape gates enforced here (Rust enforces them via generated types):
 * `nonce` must be ≥ 16 chars and `protocol_version` must equal 1.
 */

import type { ConnectHello, HostCapabilityManifest } from "@42ch/spoke-schemas";

import {
  base64UrlDecode,
  base64UrlEncode,
  getPublicKeyEd25519,
  signEd25519,
  verifyEd25519,
} from "../crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../identity.js";
import { canonicalHelloBytes } from "../jcs.js";
import { CoreError } from "./error.js";
import { PROTOCOL_VERSION } from "./version.js";

/** JCS-canonicalize the signed hello object, mapping failures to `jcs`. */
function canonicalBytes(
  peerId: string,
  nonce: string,
  host: HostCapabilityManifest,
  peerNonce?: string,
): Uint8Array {
  try {
    return canonicalHelloBytes(peerId, nonce, host, peerNonce);
  } catch (error) {
    throw new CoreError(
      "jcs",
      error instanceof Error ? error.message : String(error),
    );
  }
}

/**
 * Sign a hello with a raw Ed25519 secret key (32-byte seed), producing the
 * full `ConnectHello` wire envelope.
 *
 * `peer_id` is derived from the public key of `secret`; the hello
 * `protocol_version` is the core protocol version. The `nonce` must meet the
 * wire floor (minLength 16) or `invalid_nonce` is thrown.
 *
 * Role: `peerNonce` is `undefined` for the **initiator** hello (4-field
 * signed object — the byte-identical pre-dial-binding path; the wire
 * envelope omits `peer_nonce` entirely) and the initiator's nonce for the
 * **responder** hello (5-field signed object, dial binding).
 */
export async function signHelloEd25519(
  secret: Uint8Array,
  nonce: string,
  manifest: HostCapabilityManifest,
  peerNonce?: string,
): Promise<ConnectHello> {
  if (secret.length !== 32) {
    throw new CoreError("crypto", "secret must be 32 bytes");
  }
  if (nonce.length < 16) {
    throw new CoreError("invalid_nonce", `nonce must be at least 16 characters (got ${nonce.length})`);
  }
  if (peerNonce !== undefined && peerNonce.length < 16) {
    throw new CoreError("invalid_nonce", `peer_nonce must be at least 16 characters (got ${peerNonce.length})`);
  }
  const publicKey = getPublicKeyEd25519(secret);
  const peerId = derivePeerIdFromEd25519Pubkey(publicKey);
  const bytes = canonicalBytes(peerId, nonce, manifest, peerNonce);
  const signature = base64UrlEncode(await signEd25519(secret, bytes));

  return {
    protocol_version: PROTOCOL_VERSION,
    peer_id: peerId,
    nonce,
    ...(peerNonce !== undefined ? { peer_nonce: peerNonce } : {}),
    host: manifest,
    signature,
    extensions: {},
  };
}

/**
 * Verify a received hello against a raw Ed25519 public key (32 bytes).
 *
 * Checks, in order:
 * 1. `protocol_version` equals the core protocol version
 *    (`protocol_version_mismatch` — a mixed-version peer or downgrade
 *    attempt; the dedicated kind, not `handshake_failed`).
 * 2. `nonce` meets the wire floor (minLength 16).
 * 3. The verify key derives `expectedPeerId` — the authenticated remote
 *    peer. A key that derives a different peer id cannot attest that peer's
 *    identity.
 * 4. The claimed `hello.peer_id` equals `expectedPeerId`.
 * 5. The signature verifies over the JCS-canonicalized signed object.
 *
 * Role-aware (dial binding): a hello carrying `peer_nonce` (responder) is
 * verified over the 5-field signed object, and when `expectedPeerNonce` is
 * supplied the initiator asserts the responder's `peer_nonce` equals its own
 * nonce — a replayed responder hello (signed over a different or absent
 * initiator nonce) fails here, which is the cross-restart replay defense.
 * Fail-closed: when `expectedPeerNonce` is supplied (the dial expects a
 * responder), a hello WITHOUT `peer_nonce` is rejected — the mixed-version
 * downgrade (an old responder omits the field) must not bypass the binding.
 * A hello without `peer_nonce` is verified over the 4-field signed object
 * only when no `expectedPeerNonce` is asserted.
 *
 * Allowlist and nonce gates are separate core checks (`allowlist` and
 * `nonce` modules).
 */
export async function verifyHelloEd25519(
  publicKey: Uint8Array,
  expectedPeerId: string,
  hello: ConnectHello,
  expectedPeerNonce?: string,
): Promise<void> {
  if (hello.protocol_version !== PROTOCOL_VERSION) {
    throw new CoreError(
      "protocol_version_mismatch",
      `unsupported protocol_version ${hello.protocol_version} (expected ${PROTOCOL_VERSION})`,
    );
  }
  if (hello.nonce.length < 16) {
    throw new CoreError("invalid_nonce", `hello nonce must be at least 16 characters (got ${hello.nonce.length})`);
  }
  if (hello.peer_nonce !== undefined && hello.peer_nonce.length < 16) {
    throw new CoreError("invalid_nonce", `hello peer_nonce must be at least 16 characters (got ${hello.peer_nonce.length})`);
  }
  if (publicKey.length !== 32) {
    throw new CoreError("crypto", "public key must be 32 bytes");
  }
  const derivedPeerId = derivePeerIdFromEd25519Pubkey(publicKey);
  if (derivedPeerId !== expectedPeerId) {
    throw new CoreError(
      "handshake_failed",
      `public key derives peer id ${derivedPeerId} instead of the authenticated peer ${expectedPeerId}`,
    );
  }
  if (hello.peer_id !== expectedPeerId) {
    throw new CoreError(
      "handshake_failed",
      `hello peer_id ${hello.peer_id} does not match authenticated peer ${expectedPeerId}`,
    );
  }

  // The claimed peer id was just checked to equal `expectedPeerId`, so
  // canonicalizing over `expectedPeerId` reproduces the signer's bytes.
  // Role-aware signed object: 5 fields (incl. `peer_nonce`) for a responder
  // hello, 4 fields for an initiator hello. When the initiator supplies
  // `expectedPeerNonce`, the responder's `peer_nonce` must be PRESENT AND
  // equal to it — this is the dial binding that defeats cross-restart
  // replay. Fail-closed (spec §Dial binding): a dial that expects a
  // responder MUST NOT accept an initiator-shaped hello — a mixed-version
  // old responder omits `peer_nonce`, and silently falling back to the
  // 4-field verify would skip the binding assert entirely.
  let bytes: Uint8Array;
  if (hello.peer_nonce !== undefined) {
    if (
      expectedPeerNonce !== undefined &&
      hello.peer_nonce !== expectedPeerNonce
    ) {
      throw new CoreError(
        "handshake_failed",
        `responder hello peer_nonce does not match the initiator nonce (dial binding)`,
      );
    }
    bytes = canonicalBytes(
      expectedPeerId,
      hello.nonce,
      hello.host,
      hello.peer_nonce,
    );
  } else {
    if (expectedPeerNonce !== undefined) {
      throw new CoreError(
        "handshake_failed",
        "responder hello is missing peer_nonce (dial binding)",
      );
    }
    bytes = canonicalBytes(expectedPeerId, hello.nonce, hello.host);
  }

  let signature: Uint8Array;
  try {
    signature = base64UrlDecode(hello.signature);
  } catch {
    throw new CoreError("invalid_hello_signature");
  }
  if (signature.length !== 64) {
    throw new CoreError("invalid_hello_signature");
  }
  const ok = await verifyEd25519(publicKey, bytes, signature);
  if (!ok) {
    throw new CoreError("invalid_hello_signature");
  }
}
