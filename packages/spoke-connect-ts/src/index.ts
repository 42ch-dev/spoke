/**
 * @42ch/spoke-connect-ts — SPOKE connect client library (workspace-private).
 *
 * Public surface (AD-P0-6): peer_id identity derivation, Ed25519 sign/verify
 * (WebCrypto primary with @noble/ed25519 fallback), RFC 8785 JCS hello
 * canonicalization over the `@42ch/spoke-schemas` connect wire types, and
 * one-JSON-per-message WebSocket framing (AD-P0-3).
 *
 * Forced-`@noble` helpers (`signEd25519Noble` / `verifyEd25519Noble`) remain
 * exported from `./crypto.js` for tests and fallback verification, but are
 * not part of the client surface. The Node `ws` adapter lives in
 * `src/node/ws.ts` (Node-only subpath, AD-P0-5) and is not exported here.
 */

export { derivePeerIdFromEd25519Pubkey } from "./identity.js";

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
