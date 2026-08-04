#!/usr/bin/env node
/**
 * Built-package published-shape smoke (connect-ts-noise-stack QC wave, W2).
 *
 * Resolves `@42ch/spoke-connect` and `@42ch/spoke-connect/noise` against the
 * BUILT `dist/` through the package `exports` map (Node self-reference) —
 * deliberately NOT the vitest source aliases. A dropped `require` condition,
 * a missing dist file, or a wrong conditional target fails here exactly as
 * it would for a packed consumer.
 *
 * Prerequisite: `pnpm --filter @42ch/spoke-connect build` (and a built
 * `@42ch/spoke-schemas`, which CI builds earlier in the same job).
 *
 * Run: `pnpm --filter @42ch/spoke-connect test:dist`
 */
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

/** Root (default bundle) symbols that must resolve in both module systems. */
const DEFAULT_SESSION_SYMBOLS = ["derivePeerIdFromEd25519Pubkey"];
/** Noise symbols that must NOT leak into the default root bundle. */
const NOISE_FORBIDDEN_IN_ROOT = [
  "NoiseXX",
  "NoiseHandshake",
  "NoiseTransport",
  "NOISE_PROTOCOL_ID",
];
/** `./noise` barrel symbols (src/noise/index.ts re-export table). */
const NOISE_BARREL_SYMBOLS = [
  "NOISE_PROTOCOL_ID",
  "NOISE_PROTOCOL_NAME",
  "MAX_NOISE_MSG_LEN",
  "MAX_FRAME_LEN",
  "STATIC_KEY_DOMAIN",
  "createNoiseStaticKeypair",
  "encodeLengthPrefixed",
  "decodeLengthPrefixed",
  "encodeHandshakePayload",
  "decodeHandshakePayload",
  "signNoiseStaticKey",
  "verifyNoiseStaticKey",
  "NoiseXX",
  "NoiseHandshake",
  "NoiseTransport",
];
/** Internal raw-crypto wrappers that must stay out of the public barrel. */
const NOISE_FORBIDDEN_IN_BARREL = ["encrypt", "decrypt", "dh", "hkdf"];

// ── ESM self-reference ("import" conditions) ───────────────────────────────
const rootEsm = await import("@42ch/spoke-connect");
const noiseEsm = await import("@42ch/spoke-connect/noise");

// ── CJS self-reference ("require" conditions) ──────────────────────────────
const rootCjs = require("@42ch/spoke-connect");
const noiseCjs = require("@42ch/spoke-connect/noise");

for (const [label, mod] of [
  ["ESM", rootEsm],
  ["CJS", rootCjs],
]) {
  for (const sym of DEFAULT_SESSION_SYMBOLS) {
    assert.equal(
      typeof mod[sym],
      "function",
      `${label} root: default-session symbol ${sym} must resolve`,
    );
  }
  for (const sym of NOISE_FORBIDDEN_IN_ROOT) {
    assert.equal(
      mod[sym],
      undefined,
      `${label} root: Noise symbol ${sym} must not leak into the default bundle`,
    );
  }
}

for (const [label, mod] of [
  ["ESM", noiseEsm],
  ["CJS", noiseCjs],
]) {
  for (const sym of NOISE_BARREL_SYMBOLS) {
    assert.ok(
      mod[sym] !== undefined,
      `${label} ./noise: barrel symbol ${sym} must resolve`,
    );
  }
  for (const sym of NOISE_FORBIDDEN_IN_BARREL) {
    assert.equal(
      mod[sym],
      undefined,
      `${label} ./noise: internal raw crypto ${sym} must not be exported`,
    );
  }
}

console.log(
  "dist-shape smoke PASS: `.` and `./noise` resolve in ESM + CJS; " +
    "root excludes Noise; noise barrel surface intact.",
);
