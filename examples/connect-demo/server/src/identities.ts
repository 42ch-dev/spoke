/**
 * DEMO ONLY — never reuse.
 *
 * Fixed Ed25519 identity seeds for the connect demo (server, client, and a
 * "stranger" peer that is deliberately NOT on the server allowlist). The
 * seeds and everything derived from them (public keys, peer ids) are pure
 * constants — the negative-proof test depends on the stranger's peer id
 * being stable across runs.
 */

import {
  derivePeerIdFromEd25519Pubkey,
  getPublicKeyEd25519,
} from "@42ch/spoke-connect";

/** DEMO ONLY — never reuse: server Ed25519 seed. */
export const DEMO_SERVER_SEED = new Uint8Array(32).fill(0x51);

/** DEMO ONLY — never reuse: client Ed25519 seed. */
export const DEMO_CLIENT_SEED = new Uint8Array(32).fill(0x63);

/** DEMO ONLY — never reuse: non-allowlisted stranger Ed25519 seed. */
export const DEMO_STRANGER_SEED = new Uint8Array(32).fill(0x73);

/** Public key derived from {@link DEMO_SERVER_SEED}. */
export const DEMO_SERVER_PUBKEY = getPublicKeyEd25519(DEMO_SERVER_SEED);

/** Public key derived from {@link DEMO_CLIENT_SEED}. */
export const DEMO_CLIENT_PUBKEY = getPublicKeyEd25519(DEMO_CLIENT_SEED);

/** Public key derived from {@link DEMO_STRANGER_SEED}. */
export const DEMO_STRANGER_PUBKEY = getPublicKeyEd25519(DEMO_STRANGER_SEED);

/** peer_id of the demo server (derived from {@link DEMO_SERVER_PUBKEY}). */
export const DEMO_SERVER_PEER_ID = derivePeerIdFromEd25519Pubkey(
  DEMO_SERVER_PUBKEY,
);

/** peer_id of the demo client (derived from {@link DEMO_CLIENT_PUBKEY}). */
export const DEMO_CLIENT_PEER_ID = derivePeerIdFromEd25519Pubkey(
  DEMO_CLIENT_PUBKEY,
);

/** peer_id of the stranger (derived from {@link DEMO_STRANGER_PUBKEY}). */
export const DEMO_STRANGER_PEER_ID = derivePeerIdFromEd25519Pubkey(
  DEMO_STRANGER_PUBKEY,
);
