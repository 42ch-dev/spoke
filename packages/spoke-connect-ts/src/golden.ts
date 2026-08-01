/**
 * Golden vectors for SPOKE connect identity — redeclared once with
 * provenance (AD-P0-4). Tests assert against these committed constants,
 * never values recomputed only by the code under test.
 *
 * Sources (keep values byte-identical):
 * - `crates/spoke-connect/src/core/hello_crypto.rs` — GOLDEN_SEED,
 *   GOLDEN_PUBKEY, GOLDEN_PEER_ID, GOLDEN_JCS_HEX, GOLDEN_SIGNATURE test
 *   constants, captured from libp2p-identity 0.2.14 / libp2p 0.56 before
 *   the transport cutover.
 * - `crates/spoke-connect/src/core/peer_id.rs` — GOLDEN_PUBKEY,
 *   GOLDEN_PEER_ID.
 * - `tooling/connect-identity-proof/proof.mjs` — the JS reproducibility
 *   proof (6/6 PASS) using the same constants; the derivation this package
 *   ports.
 */

import type { HostCapabilityManifest } from "@42ch/spoke-schemas";

/** Ed25519 seed bytes 1..=32. */
export const GOLDEN_SEED: Uint8Array = Uint8Array.from(
  { length: 32 },
  (_, i) => i + 1,
);

/** Ed25519 public key for GOLDEN_SEED. */
export const GOLDEN_PUBKEY: Uint8Array = Uint8Array.from([
  0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9, 0x40, 0x78, 0xb1, 0x12, 0xe8,
  0xa9, 0x8b, 0xa7, 0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7, 0xe0, 0xe3,
  0x91, 0x0b, 0xad, 0x04, 0x96, 0x64,
]);

/** Peer id derived from GOLDEN_PUBKEY (base58btc identity multihash). */
export const GOLDEN_PEER_ID: string =
  "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf";

/** Golden hello nonce (16+ chars, wire floor). */
export const GOLDEN_NONCE: string = "golden-nonce-000000000001";

/**
 * RFC 8785 JCS UTF-8 bytes (hex) of the signed object
 * `{protocol_version, peer_id, nonce, host}` for the golden manifest —
 * 264 bytes.
 */
export const GOLDEN_JCS_HEX: string =
  "7b22686f7374223a7b226361706162696c6974696573223a5b2273706f6b652d626173656c696e65225d2c22657874656e73696f6e73223a7b7d2c22686f73745f6964223a22676f6c64656e2d686f7374222c226e616d65737061636573223a5b5d2c22726f6c6573223a5b22646174612d73746f7265225d2c22736368656d615f76657273696f6e223a317d2c226e6f6e6365223a22676f6c64656e2d6e6f6e63652d303030303030303030303031222c22706565725f6964223a22313244334b6f6f574a315473696a48374835463734686641443558697368517a337378726d41745659333747744e643943715966222c2270726f746f636f6c5f76657273696f6e223a317d";

/** base64url (no padding) of the raw 64-byte Ed25519 signature over GOLDEN_JCS_HEX. */
export const GOLDEN_SIGNATURE: string =
  "yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg";

/**
 * Golden host manifest — `authority` is absent (omit, never `null`), the
 * only non-required manifest member. The canonical bytes in GOLDEN_JCS_HEX
 * contain no `"authority"` member at all.
 */
export function goldenManifest(): HostCapabilityManifest {
  return {
    capabilities: ["spoke-baseline"],
    extensions: {},
    host_id: "golden-host",
    // The golden fixture carries an empty namespace list (Rust `Vec::new()`
    // → `"namespaces":[]` in GOLDEN_JCS_HEX). The generated wire type
    // enforces minItems 1 for namespaces, so the fixture diverges at the
    // type level — the JCS bytes are the pinned contract.
    namespaces: [] as unknown as [string, ...string[]],
    roles: ["data-store"],
    schema_version: 1,
  };
}
