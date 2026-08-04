/**
 * `@42ch/spoke-connect/noise` — opt-in Noise XX mesh transport subpath.
 *
 * Integrators that need a direct libp2p-noise transport (rather than
 * envelope-over-WebSocket) import this subpath; the default `.` and
 * `./node` exports never resolve these modules, so `@noble/ciphers` and
 * `@noble/curves` stay out of the thin default bundle.
 *
 * Public surface is the frozen export table (plan "Global Constraints" /
 * noise-subpath rationale). The raw crypto wrappers (`crypto.ts`) are
 * internal; everything else an integrator needs for the handshake,
 * framing, and identity binding is exported here.
 */
export {
  NOISE_PROTOCOL_ID,
  MAX_NOISE_MSG_LEN,
  MAX_FRAME_LEN,
  createNoiseStaticKeypair,
  encodeLengthPrefixed,
  decodeLengthPrefixed,
  NoiseHandshake,
  NoiseTransport,
} from "./framing.js";
export type {
  NoiseIdentity,
  NoiseHandshakeOptions,
  NoiseHandshakeResult,
  NoiseTransportKeys,
} from "./framing.js";

export {
  STATIC_KEY_DOMAIN,
  encodeHandshakePayload,
  decodeHandshakePayload,
  signNoiseStaticKey,
  verifyNoiseStaticKey,
} from "./payload.js";
export type {
  NoiseExtensions,
  NoiseHandshakePayload,
} from "./payload.js";

export { NOISE_PROTOCOL_NAME, NoiseXX } from "./xx.js";
export type { NoiseKeyPair, NoiseXXOptions, NoiseXXResult } from "./xx.js";
