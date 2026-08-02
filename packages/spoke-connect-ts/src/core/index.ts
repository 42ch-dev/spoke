/**
 * SPOKE connect session-core barrel (mirrors the `pub use` block of
 * `crates/spoke-connect/src/core/mod.rs`).
 *
 * `derivePeerIdFromEd25519Pubkey` intentionally lives outside this barrel —
 * it is already exported from `src/identity.ts` (AD-P0-3: "already in
 * identity.ts — re-export"), so the root barrel re-exports it from there.
 */

export { PROTOCOL_VERSION } from "./version.js";
export { CoreError, CoreInvokeError } from "./error.js";
export type { CoreErrorCode, CoreInvokeErrorCode } from "./error.js";
export {
  MAX_SEQUENCE,
  OutboundSequence,
  InboundSequence,
} from "./sequence.js";
export {
  checkResponseCorrelation,
  correlationFromRequest,
  correlationFromResponse,
} from "./correlate.js";
export type { Correlation } from "./correlate.js";
export {
  CAPABILITY_L2_COMPUTABLE,
  CAPABILITY_SPOKE_BASELINE,
  dispatchAllowed,
  requiredCapability,
} from "./dispatch.js";
export { generateNonce, NonceStore } from "./nonce.js";
export { isAllowlisted } from "./allowlist.js";
export { signHelloEd25519, verifyHelloEd25519 } from "./hello.js";
export { negotiatedCapabilities, Session } from "./session.js";
export type { SessionOptions } from "./session.js";
