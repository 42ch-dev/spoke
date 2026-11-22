/**
 * `noise-peerid` allowlist check (fail-closed).
 *
 * Ported from `crates/spoke-connect/src/core/allowlist.rs`; see
 * `.agents/specs/connect/spoke-connect.md` contract §Auth model: the trust root is a
 * deployment-configured peer id allowlist. An empty allowlist rejects every
 * remote peer.
 */

/** Whether `peerId` is on the allowlist. An empty allowlist rejects every peer (fail-closed). */
export function isAllowlisted(allowlist: readonly string[], peerId: string): boolean {
  // simplify: linear scan; upgrade: switch to a Set if allowlists grow past a
  // handful of peers.
  return allowlist.includes(peerId);
}
