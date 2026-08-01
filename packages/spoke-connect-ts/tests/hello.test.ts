import { describe, expect, it } from "vitest";

import { getPublicKeyEd25519 } from "../src/crypto.js";
import {
  GOLDEN_NONCE,
  GOLDEN_PEER_ID,
  GOLDEN_PUBKEY,
  GOLDEN_SEED,
  GOLDEN_SIGNATURE,
  goldenManifest,
} from "../src/golden.js";
import { derivePeerIdFromEd25519Pubkey } from "../src/identity.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../src/core/hello.js";

describe("signHelloEd25519 / verifyHelloEd25519 (port of hello_crypto.rs)", () => {
  it("signs the golden hello to the golden signature (full envelope)", async () => {
    const hello = await signHelloEd25519(GOLDEN_SEED, GOLDEN_NONCE, goldenManifest());
    expect(hello.protocol_version).toBe(1);
    expect(hello.peer_id).toBe(GOLDEN_PEER_ID);
    expect(hello.nonce).toBe(GOLDEN_NONCE);
    expect(hello.signature).toBe(GOLDEN_SIGNATURE);
  });

  it("verifies the golden hello with the raw public key", async () => {
    const hello = await signHelloEd25519(GOLDEN_SEED, GOLDEN_NONCE, goldenManifest());
    await expect(verifyHelloEd25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, hello)).resolves.toBeUndefined();
  });

  it("round-trips sign then verify with a fresh key", async () => {
    const secret = new Uint8Array(32).fill(7);
    const publicKey = getPublicKeyEd25519(secret);
    const peerId = derivePeerIdFromEd25519Pubkey(publicKey);
    const hello = await signHelloEd25519(secret, "round-trip-nonce-12345678", goldenManifest());
    expect(hello.peer_id).toBe(peerId);
    await expect(verifyHelloEd25519(publicKey, peerId, hello)).resolves.toBeUndefined();
  });

  it("rejects a short nonce at sign time (wire floor, not a panic)", async () => {
    await expect(
      signHelloEd25519(new Uint8Array(32).fill(8), "short", goldenManifest()),
    ).rejects.toThrowError(expect.objectContaining({ code: "invalid_nonce" }));
  });

  it("rejects a short nonce at verify time", async () => {
    const hello = await signHelloEd25519(GOLDEN_SEED, GOLDEN_NONCE, goldenManifest());
    const tampered = { ...hello, nonce: "short" };
    await expect(
      verifyHelloEd25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, tampered),
    ).rejects.toThrowError(expect.objectContaining({ code: "invalid_nonce" }));
  });

  it("rejects an unsupported protocol_version at verify time", async () => {
    const hello = await signHelloEd25519(GOLDEN_SEED, GOLDEN_NONCE, goldenManifest());
    const tampered = { ...hello, protocol_version: 2 };
    await expect(
      verifyHelloEd25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, tampered),
    ).rejects.toThrowError(expect.objectContaining({ code: "handshake_failed" }));
  });

  it("rejects a tampered host manifest", async () => {
    const hello = await signHelloEd25519(GOLDEN_SEED, GOLDEN_NONCE, goldenManifest());
    const tampered = {
      ...hello,
      host: {
        ...hello.host,
        roles: [...hello.host.roles, "checker"] as [string, ...string[]],
      },
    };
    await expect(
      verifyHelloEd25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, tampered),
    ).rejects.toThrowError(expect.objectContaining({ code: "invalid_hello_signature" }));
  });

  it("rejects a verify key that derives a different peer id", async () => {
    const otherPubkey = getPublicKeyEd25519(new Uint8Array(32).fill(9));
    const hello = await signHelloEd25519(GOLDEN_SEED, GOLDEN_NONCE, goldenManifest());
    await expect(
      verifyHelloEd25519(otherPubkey, GOLDEN_PEER_ID, hello),
    ).rejects.toThrowError(expect.objectContaining({ code: "handshake_failed" }));
  });

  it("rejects a claimed peer_id that does not match the authenticated peer", async () => {
    const otherPubkey = getPublicKeyEd25519(new Uint8Array(32).fill(10));
    const otherPeerId = derivePeerIdFromEd25519Pubkey(otherPubkey);
    const hello = await signHelloEd25519(GOLDEN_SEED, GOLDEN_NONCE, goldenManifest());
    await expect(
      verifyHelloEd25519(GOLDEN_PUBKEY, otherPeerId, hello),
    ).rejects.toThrowError(expect.objectContaining({ code: "handshake_failed" }));
  });

  it("rejects a malformed signature", async () => {
    const hello = await signHelloEd25519(GOLDEN_SEED, GOLDEN_NONCE, goldenManifest());
    const tampered = { ...hello, signature: "%%%not-base64url%%%" };
    await expect(
      verifyHelloEd25519(GOLDEN_PUBKEY, GOLDEN_PEER_ID, tampered),
    ).rejects.toThrowError(expect.objectContaining({ code: "invalid_hello_signature" }));
  });
});
