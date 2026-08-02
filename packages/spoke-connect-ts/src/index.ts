/**
 * @42ch/spoke-connect-ts — SPOKE connect client library (workspace-private).
 *
 * Public surface (AD-P0-6): peer_id identity derivation, Ed25519 sign/verify
 * (WebCrypto primary with @noble/ed25519 fallback), RFC 8785 JCS hello
 * canonicalization over the `@42ch/spoke-schemas` connect wire types,
 * one-JSON-per-message WebSocket framing (AD-P0-3), and the pure
 * session-core port (`src/core/`, AD-P0-3): hello sign/verify, per-direction
 * sequence counters, response correlation, op dispatch gate, nonce replay
 * store, allowlist, and the thin `Session` helper.
 *
 * Forced-`@noble` helpers (`signEd25519Noble` / `verifyEd25519Noble`) remain
 * exported from `./crypto.js` for tests and fallback verification, but are
 * not part of the client surface. The Node `ws` adapter and the minimal
 * `connectClient` live under `src/node/` (Node-only subpath, AD-P0-5) and
 * are not exported here.
 */

export {
  derivePeerIdFromEd25519Pubkey,
  ed25519PubkeyFromPeerId,
} from "./identity.js";

export {
  base64UrlDecode,
  base64UrlEncode,
  ed25519PubkeyToSpki,
  ed25519SeedToPkcs8,
  getPublicKeyEd25519,
  signEd25519,
  verifyEd25519,
  webcryptoEd25519Available,
} from "./crypto.js";

export {
  decodeJsonMessage,
  encodeJsonMessage,
} from "./framing.js";

export { canonicalHelloBytes } from "./jcs.js";

export * from "./core/index.js";
