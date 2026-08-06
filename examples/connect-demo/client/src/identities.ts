/**
 * DEMO ONLY — never reuse.
 *
 * Client-side demo identity constants. These are the SAME fixed seeds the
 * server package ships in `server/src/identities.ts` — the third-party
 * client keeps its own copy because it must not import the demo server
 * package at runtime (dep-surface story: only `@42ch/spoke-connect` +
 * `@42ch/spoke-schemas` + `ws`). The e2e catches any drift: the server
 * allowlist and this client identity must agree for the dial to succeed.
 */

import {
  derivePeerIdFromEd25519Pubkey,
  getPublicKeyEd25519,
} from "@42ch/spoke-connect";

/** DEMO ONLY — never reuse: server Ed25519 seed (matches server/src/identities.ts). */
export const DEMO_SERVER_SEED = new Uint8Array(32).fill(0x51);

/** DEMO ONLY — never reuse: client Ed25519 seed (matches server/src/identities.ts). */
export const DEMO_CLIENT_SEED = new Uint8Array(32).fill(0x63);

/** DEMO ONLY — never reuse: non-allowlisted stranger Ed25519 seed (matches server/src/identities.ts). */
export const DEMO_STRANGER_SEED = new Uint8Array(32).fill(0x73);

/** Public key derived from {@link DEMO_SERVER_SEED} — the remote key the client trusts. */
export const DEMO_SERVER_PUBKEY = getPublicKeyEd25519(DEMO_SERVER_SEED);

/** peer_id derived from {@link DEMO_SERVER_PUBKEY} — the client's allowlist entry. */
export const DEMO_SERVER_PEER_ID = derivePeerIdFromEd25519Pubkey(
  DEMO_SERVER_PUBKEY,
);

/** The client's own peer_id (derived; printed by the CLI). */
export const DEMO_CLIENT_PEER_ID = derivePeerIdFromEd25519Pubkey(
  getPublicKeyEd25519(DEMO_CLIENT_SEED),
);

/** The demo namespace — shared with the server's seed corpus (`demo-harbor`). */
export const DEMO_SCOPE_ID = "demo-harbor";
