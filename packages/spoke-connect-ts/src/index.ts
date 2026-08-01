/**
 * @42ch/spoke-connect-ts — SPOKE connect client library (workspace-private).
 *
 * Public surface: peer_id identity derivation, Ed25519 sign/verify (WebCrypto
 * primary with @noble/ed25519 fallback), and RFC 8785 JCS hello
 * canonicalization over the `@42ch/spoke-schemas` connect wire types.
 */

export { derivePeerIdFromEd25519Pubkey } from "./identity.js";

export {
  base64UrlDecode,
  base64UrlEncode,
  ed25519PubkeyToSpki,
  ed25519SeedToPkcs8,
  getPublicKeyEd25519,
  signEd25519,
  signEd25519Noble,
  verifyEd25519,
  verifyEd25519Noble,
  webcryptoEd25519Available,
} from "./crypto.js";

export { canonicalHelloBytes } from "./jcs.js";
