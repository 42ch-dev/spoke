/**
 * Single-use `(peer_id, nonce)` replay store + hello nonce generator.
 *
 * Ported from `crates/spoke-connect/src/core/nonce.rs`; normative rules
 * `.mstar/specs/spoke-connect.md` §Nonce / replay protection: nonce
 * uniqueness is scoped per **sender** `peer_id`; a receiver MUST reject a
 * hello whose `(peer_id, nonce)` pair was already accepted. An in-memory set
 * for the life of the process is sufficient for protocol v1.
 *
 * The store only records **accepted** hellos — the caller must not call
 * `checkAndRecord` for a hello that failed an earlier gate, so a rejected
 * hello stays retry-safe.
 */

import { base64UrlEncode } from "../crypto.js";

/**
 * In-memory `(peer_id, nonce)` store of accepted hellos. The composite key
 * uses a NUL separator — peer ids (base58btc) and nonces (base64url/hex)
 * never contain NUL, so the pair is unambiguous.
 */
export class NonceStore {
  private readonly seen = new Set<string>();

  /**
   * Records `(peerId, nonce)` unless it was already accepted; returns
   * `false` on replay. Call only after the hello passed every earlier gate
   * (allowlist, signature) so a rejected hello is not burned.
   */
  checkAndRecord(peerId: string, nonce: string): boolean {
    const key = `${peerId}\u0000${nonce}`;
    if (this.seen.has(key)) {
      return false;
    }
    this.seen.add(key);
    return true;
  }
}

/**
 * Fresh hello nonce: 16 CSPRNG bytes encoded base64url without padding
 * (22 chars, ≥ the minLength 16 wire floor; ≥128 bits entropy as the spec
 * recommends for generators).
 */
export function generateNonce(): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  return base64UrlEncode(bytes);
}
