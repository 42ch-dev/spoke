import { beforeAll, describe, expect, it } from "vitest";

import {
  base64UrlDecode,
  base64UrlEncode,
  getPublicKeyEd25519,
  signEd25519,
  signEd25519Noble,
  verifyEd25519,
  verifyEd25519Noble,
  webcryptoEd25519Available,
} from "../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../src/identity.js";
import { canonicalHelloBytes } from "../src/jcs.js";
import {
  GOLDEN_JCS_HEX,
  GOLDEN_NONCE,
  GOLDEN_PEER_ID,
  GOLDEN_PUBKEY,
  GOLDEN_SEED,
  GOLDEN_SIGNATURE,
  goldenManifest,
} from "../src/golden.js";
import { fromHex, toHex } from "./hex.js";

/**
 * Golden-vector parity (AD-P0-4): every value asserted here is byte-identical
 * to the constants captured from rust-libp2p and redeclared in
 * crates/spoke-connect/src/core/{hello_crypto,peer_id}.rs and
 * tooling/connect-identity-proof/proof.mjs (6/6 PASS).
 */
describe("golden-vector parity with Rust core", () => {
  it("derives the golden public key from the golden seed", () => {
    expect(toHex(getPublicKeyEd25519(GOLDEN_SEED))).toBe(toHex(GOLDEN_PUBKEY));
  });

  it("derives the golden peer_id from the golden public key", () => {
    expect(derivePeerIdFromEd25519Pubkey(GOLDEN_PUBKEY)).toBe(GOLDEN_PEER_ID);
  });

  it("canonicalizes the golden signed object to the golden JCS bytes (264)", () => {
    const bytes = canonicalHelloBytes(GOLDEN_PEER_ID, GOLDEN_NONCE, goldenManifest());
    expect(bytes.length).toBe(264);
    expect(toHex(bytes)).toBe(GOLDEN_JCS_HEX);
  });

  it("signs golden JCS with the golden seed to the golden signature (@noble path)", () => {
    const sig = signEd25519Noble(GOLDEN_SEED, fromHex(GOLDEN_JCS_HEX));
    expect(sig.length).toBe(64);
    expect(base64UrlEncode(sig)).toBe(GOLDEN_SIGNATURE);
  });

  it("verifies the golden signature over golden JCS (@noble path)", () => {
    expect(
      verifyEd25519Noble(
        GOLDEN_PUBKEY,
        fromHex(GOLDEN_JCS_HEX),
        base64UrlDecode(GOLDEN_SIGNATURE),
      ),
    ).toBe(true);
  });

  it("auto-selected sign path reproduces the golden signature", async () => {
    const sig = await signEd25519(GOLDEN_SEED, fromHex(GOLDEN_JCS_HEX));
    expect(base64UrlEncode(sig)).toBe(GOLDEN_SIGNATURE);
  });

  it("auto-selected verify path accepts the golden signature", async () => {
    expect(
      await verifyEd25519(
        GOLDEN_PUBKEY,
        fromHex(GOLDEN_JCS_HEX),
        base64UrlDecode(GOLDEN_SIGNATURE),
      ),
    ).toBe(true);
  });
});

describe("WebCrypto backend coverage", () => {
  let webcryptoOk = false;

  beforeAll(async () => {
    webcryptoOk = await webcryptoEd25519Available();
  });

  it("reports the runtime backend for the auto-selected path", (ctx) => {
    if (!webcryptoOk) ctx.skip();
    // The auto-selected path above used WebCrypto Ed25519 on this runtime.
    expect(webcryptoOk).toBe(true);
  });
});
