#!/usr/bin/env node
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

const DEFAULT_SESSION_SYMBOLS = ["derivePeerIdFromEd25519Pubkey"];
const NOISE_FORBIDDEN_IN_ROOT = [
  "NoiseXX",
  "NoiseHandshake",
  "NoiseTransport",
  "NOISE_PROTOCOL_ID",
];
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
const NOISE_FORBIDDEN_IN_BARREL = ["encrypt", "decrypt", "dh", "hkdf"];
const REMOTE_BARREL_SYMBOLS = [
  "connectRemoteAdapter",
  "RemoteAdapter",
  "loopbackTransportPair",
  "LoopbackTransport",
  "isConnectHello",
  "isConnectInvokeRequest",
  "isConnectInvokeResponse",
  "isConnectSession",
];
const REMOTE_FORBIDDEN_IN_ROOT = ["connectRemoteAdapter", "RemoteAdapter"];

const rootEsm = await import("@42ch/spoke-connect");
const noiseEsm = await import("@42ch/spoke-connect/noise");
const remoteEsm = await import("@42ch/spoke-connect/remote");

const rootCjs = require("@42ch/spoke-connect");
const noiseCjs = require("@42ch/spoke-connect/noise");
const remoteCjs = require("@42ch/spoke-connect/remote");

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
  for (const sym of REMOTE_FORBIDDEN_IN_ROOT) {
    assert.equal(
      mod[sym],
      undefined,
      `${label} root: RemoteAdapter symbol ${sym} must not leak into the default bundle`,
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

for (const [label, mod] of [
  ["ESM", remoteEsm],
  ["CJS", remoteCjs],
]) {
  for (const sym of REMOTE_BARREL_SYMBOLS) {
    assert.ok(
      mod[sym] !== undefined,
      `${label} ./remote: barrel symbol ${sym} must resolve`,
    );
  }
}

console.log(
  "dist-shape smoke PASS: `.`, `./noise` and `./remote` resolve in ESM + CJS; " +
    "root excludes Noise; noise barrel surface intact; remote barrel surface intact.",
);
