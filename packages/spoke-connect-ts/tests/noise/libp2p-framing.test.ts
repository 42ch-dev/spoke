/**
 * libp2p Noise wire framing gates (Task 3, connect-ts-noise-stack) —
 * frozen contract `.mstar/iterations/v0-iter029/guides/noise-xx-libp2p-contract.md`
 * §3 (u16-BE length-prefix codec, max sizes) and §5 (role handshake
 * sequence), plus the `NoiseTransport` post-split AEAD frame codec and the
 * `NoiseHandshake` driver that binds `NoiseXX` + framing + identity payload.
 *
 * Byte fixtures reuse the canonical cacophony vector keys/messages from
 * `tests/noise/xx.test.ts`; the identity seeds are the golden-hello seed
 * (A) and a fixed second seed (B).
 */
import { describe, expect, it } from "vitest";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { getPublicKey } from "../../src/noise/crypto.js";
import {
  EXTRA_ENCRYPT_SPACE,
  MAX_FRAME_LEN,
  MAX_NOISE_MSG_LEN,
  NOISE_PROTOCOL_ID,
  NoiseHandshake,
  NoiseTransport,
  createNoiseStaticKeypair,
  decodeLengthPrefixed,
  encodeLengthPrefixed,
  splitLengthPrefixedFrames,
} from "../../src/noise/framing.js";
import {
  encodeHandshakePayload,
  encodeIdentityPublicKey,
  signNoiseStaticKey,
} from "../../src/noise/payload.js";
import { NoiseXX } from "../../src/noise/xx.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import { fromHex, toHex } from "../hex.js";

// ── Fixtures (cacophony vector keys, xx.test.ts) ───────────────────────────

const INIT_STATIC_PRIV = fromHex(
  "e61ef9919cde45dd5f82166404bd08e38bceb5dfdfded0a34c8df7ed542214d1",
);
const RESP_STATIC_PRIV = fromHex(
  "4a3acbfdb163dec651dfa3194dece676d437029c62a408b4c5ea9114246e4893",
);
const INIT_EPH_PRIV = fromHex(
  "893e28b9dc6ca8d611ab664754b8ceb7bac5117349a4439a6b0569da977c464a",
);
const RESP_EPH_PRIV = fromHex(
  "bbdb4cdbd309f1a1f2e1456967fe288cadd6f712d65dc7b7793d5e63da6b375b",
);
/** Flight-1 message of the vector (48 bytes): e.pub || "Ludwig von Mises". */
const VECTOR_FLIGHT1 = fromHex(
  "ca35def5ae56cec33dc2036731ab14896bc4c75dbb07a61f879f8e3afa4c79444c756477696720766f6e204d69736573",
);
/** The vector's initiator ephemeral public key (first 32 bytes). */
const FLIGHT1_E_PUB = VECTOR_FLIGHT1.subarray(0, 32);
/** "John Galt" — the cacophony vector prologue. */
const PROLOGUE = fromHex("4a6f686e2047616c74");

const SEED_A = fromHex(
  "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
); // golden-hello seed
const SEED_B = fromHex(
  "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
);
/** golden-hello.json peer_id for SEED_A. */
const PEER_ID_A = "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf";

const EMPTY = new Uint8Array(0);

// ── §3 constants ────────────────────────────────────────────────────────────

describe("framing constants", () => {
  it("match the frozen contract table", () => {
    expect(MAX_NOISE_MSG_LEN).toBe(65535);
    expect(EXTRA_ENCRYPT_SPACE).toBe(1024);
    expect(MAX_FRAME_LEN).toBe(65535 - 1024); // 64511
    expect(NOISE_PROTOCOL_ID).toBe("/noise");
  });
});

// ── §3 u16-BE length-prefix codec ───────────────────────────────────────────

describe("encodeLengthPrefixed / decodeLengthPrefixed", () => {
  it("encodes the empty payload as a zero-length frame (00 00)", () => {
    expect(toHex(encodeLengthPrefixed(EMPTY))).toBe("0000");
  });

  it("encodes a payload with a u16 big-endian length", () => {
    expect(toHex(encodeLengthPrefixed(new TextEncoder().encode("abc")))).toBe(
      "0003616263",
    );
  });

  it("frames the canonical cacophony flight-1 message byte-for-byte", () => {
    expect(VECTOR_FLIGHT1).toHaveLength(48);
    expect(toHex(encodeLengthPrefixed(VECTOR_FLIGHT1))).toBe(
      "0030" + toHex(VECTOR_FLIGHT1),
    );
  });

  it("round-trips payloads", () => {
    const payload = new Uint8Array(1024).fill(0x5a);
    expect(decodeLengthPrefixed(encodeLengthPrefixed(payload))).toEqual(payload);
  });

  it("rejects payloads over MAX_NOISE_MSG_LEN (u16 range)", () => {
    expect(encodeLengthPrefixed(new Uint8Array(MAX_NOISE_MSG_LEN))).toHaveLength(
      2 + MAX_NOISE_MSG_LEN,
    );
    expect(() => encodeLengthPrefixed(new Uint8Array(MAX_NOISE_MSG_LEN + 1))).toThrow();
  });

  it("rejects a truncated length header", () => {
    expect(() => decodeLengthPrefixed(new Uint8Array(1))).toThrow();
  });

  it("rejects a truncated payload", () => {
    expect(() => decodeLengthPrefixed(fromHex("000261"))).toThrow();
  });

  it("rejects trailing bytes after the declared payload", () => {
    expect(() => decodeLengthPrefixed(fromHex("0001616263"))).toThrow();
  });

  it("splits concatenated frames", () => {
    const frames = [
      encodeLengthPrefixed(new TextEncoder().encode("abc")),
      encodeLengthPrefixed(EMPTY),
      encodeLengthPrefixed(new Uint8Array(300).fill(0x11)),
    ];
    const split = splitLengthPrefixedFrames(fromHex(frames.map(toHex).join("")));
    expect(split).toHaveLength(3);
    expect(split.map(toHex)).toEqual(frames.map(toHex));
    expect(splitLengthPrefixedFrames(EMPTY)).toEqual([]);
  });

  it("rejects a truncated concatenated buffer", () => {
    const frame = encodeLengthPrefixed(new Uint8Array(10));
    expect(() => splitLengthPrefixedFrames(frame.subarray(0, frame.length - 1))).toThrow();
  });
});

// ── §3.1 NoiseTransport (post-split AEAD frames) ────────────────────────────

function makeTransportPair() {
  const initiator = new NoiseXX({
    initiator: true,
    staticKeyPair: { privateKey: INIT_STATIC_PRIV },
  });
  const responder = new NoiseXX({
    initiator: false,
    staticKeyPair: { privateKey: RESP_STATIC_PRIV },
  });
  const f1 = initiator.writeMessage(EMPTY);
  responder.readMessage(f1);
  const f2 = responder.writeMessage(EMPTY);
  initiator.readMessage(f2);
  const f3 = initiator.writeMessage(EMPTY);
  responder.readMessage(f3);
  const init = initiator.finish();
  const resp = responder.finish();
  return {
    init: new NoiseTransport({ tx: init.tx, rx: init.rx }),
    resp: new NoiseTransport({ tx: resp.tx, rx: resp.rx }),
  };
}

describe("NoiseTransport", () => {
  it("round-trips frames in both directions", () => {
    const { init, resp } = makeTransportPair();
    const payload = new TextEncoder().encode("spoke-over-noise");
    expect(toHex(resp.readFrame(init.writeFrame(payload)))).toBe(toHex(payload));
    expect(toHex(init.readFrame(resp.writeFrame(payload)))).toBe(toHex(payload));
  });

  it("length-prefixes each sealed frame (cleartext + 16 tag + 2 prefix)", () => {
    const { init } = makeTransportPair();
    const payload = new TextEncoder().encode("abcdefghijklmnopqrstuvwxyz");
    expect(init.writeFrame(payload)).toHaveLength(2 + payload.length + 16);
  });

  it("advances the per-direction nonce across frames", () => {
    const { init, resp } = makeTransportPair();
    const payload = new TextEncoder().encode("same-plaintext");
    const frame1 = init.writeFrame(payload);
    const frame2 = init.writeFrame(payload);
    expect(toHex(frame1)).not.toBe(toHex(frame2)); // nonce 0 vs 1
    expect(toHex(resp.readFrame(frame1))).toBe(toHex(payload));
    expect(toHex(resp.readFrame(frame2))).toBe(toHex(payload));
  });

  it("rejects a tampered frame", () => {
    const { init, resp } = makeTransportPair();
    const frame = init.writeFrame(new TextEncoder().encode("integrity"));
    frame[frame.length - 1] ^= 0x01; // inside the Poly1305 tag
    expect(() => resp.readFrame(frame)).toThrow();
  });

  it("rejects opening a frame with the wrong key", () => {
    const { init } = makeTransportPair();
    const frame = init.writeFrame(new TextEncoder().encode("own-key"));
    expect(() => init.readFrame(frame)).toThrow(); // rx ≠ tx
  });

  it("splits cleartext above MAX_FRAME_LEN across length-prefixed frames", () => {
    const { init, resp } = makeTransportPair();
    const big = new Uint8Array(MAX_FRAME_LEN + 100).fill(0xab);
    const frames = splitLengthPrefixedFrames(init.writeFrame(big));
    expect(frames).toHaveLength(2);
    expect(frames[0]).toHaveLength(2 + MAX_FRAME_LEN + 16);
    expect(frames[1]).toHaveLength(2 + 100 + 16);
    expect(resp.readFrame(frames[0])).toEqual(big.subarray(0, MAX_FRAME_LEN));
    expect(resp.readFrame(frames[1])).toEqual(big.subarray(MAX_FRAME_LEN));
  });

  it("keeps a cleartext of exactly MAX_FRAME_LEN in a single frame", () => {
    const { init, resp } = makeTransportPair();
    const exact = new Uint8Array(MAX_FRAME_LEN).fill(0xcd);
    const frames = splitLengthPrefixedFrames(init.writeFrame(exact));
    expect(frames).toHaveLength(1);
    expect(resp.readFrame(frames[0])).toEqual(exact);
  });

  it("writes no bytes for an empty payload", () => {
    const { init } = makeTransportPair();
    expect(init.writeFrame(EMPTY)).toHaveLength(0);
  });
});

// ── §5 NoiseHandshake driver ────────────────────────────────────────────────

async function makeDriverPair() {
  const initiator = new NoiseHandshake({
    initiator: true,
    staticKeyPair: { privateKey: INIT_STATIC_PRIV },
    identity: { seed: SEED_A },
  });
  const responder = new NoiseHandshake({
    initiator: false,
    staticKeyPair: { privateKey: RESP_STATIC_PRIV },
    identity: { seed: SEED_B },
  });
  return { initiator, responder };
}

async function runDriverHandshake(
  initiator: NoiseHandshake,
  responder: NoiseHandshake,
) {
  const f1 = await initiator.writeMessage();
  await responder.readMessage(f1);
  const f2 = await responder.writeMessage();
  await initiator.readMessage(f2);
  const f3 = await initiator.writeMessage();
  await responder.readMessage(f3);
}

describe("NoiseHandshake", () => {
  it("completes a full handshake and binds identities on both sides", async () => {
    const { initiator, responder } = await makeDriverPair();
    await runDriverHandshake(initiator, responder);
    const init = await initiator.finish();
    const resp = await responder.finish();

    // Both parties agree on the handshake hash.
    expect(toHex(init.handshakeHash)).toBe(toHex(resp.handshakeHash));
    // Remote static X25519 key comes from the `s` tokens.
    expect(toHex(init.remoteStaticPublic)).toBe(
      toHex(getPublicKey(RESP_STATIC_PRIV)),
    );
    expect(toHex(resp.remoteStaticPublic)).toBe(
      toHex(getPublicKey(INIT_STATIC_PRIV)),
    );
    // Remote identity comes from the payload `identity_key` (Ed25519).
    expect(toHex(init.remoteIdentityPublicKey)).toBe(
      toHex(getPublicKeyEd25519(SEED_B)),
    );
    expect(toHex(resp.remoteIdentityPublicKey)).toBe(
      toHex(getPublicKeyEd25519(SEED_A)),
    );
    // peer_id = the SPOKE derivation over the remote identity key.
    expect(init.remotePeerId).toBe(
      derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(SEED_B)),
    );
    expect(resp.remotePeerId).toBe(PEER_ID_A); // pinned by golden-hello.json

    // Post-handshake frames open across the pair in both directions.
    const payload = new TextEncoder().encode("spoke-over-noise");
    expect(toHex(resp.transport.readFrame(init.transport.writeFrame(payload)))).toBe(
      toHex(payload),
    );
    expect(toHex(init.transport.readFrame(resp.transport.writeFrame(payload)))).toBe(
      toHex(payload),
    );
  });

  it("frames the pinned flight-1 bytes with fixed keys and prologue", async () => {
    const initiator = new NoiseHandshake({
      initiator: true,
      staticKeyPair: { privateKey: INIT_STATIC_PRIV },
      identity: { seed: SEED_A },
      prologue: PROLOGUE,
      ephemeralKeyPair: { privateKey: INIT_EPH_PRIV },
    });
    const f1 = await initiator.writeMessage();
    expect(f1).toHaveLength(2 + 32);
    expect(toHex(f1)).toBe("0020" + toHex(FLIGHT1_E_PUB));
  });

  it("emits the contract flight sizes with identity payloads", async () => {
    const { initiator, responder } = await makeDriverPair();
    const f1 = await initiator.writeMessage();
    await responder.readMessage(f1);
    const f2 = await responder.writeMessage();
    await initiator.readMessage(f2);
    const f3 = await initiator.writeMessage();
    await responder.readMessage(f3);
    // payload = 104 bytes (2+36 identity_key, 2+64 identity_sig); handshake
    // wires: f1 = 32, f2 = 32+48+payload+16, f3 = 48+payload+16 (xx.test.ts).
    expect(f1).toHaveLength(2 + 32);
    expect(f2).toHaveLength(2 + 32 + 48 + 104 + 16);
    expect(f3).toHaveLength(2 + 48 + 104 + 16);
  });

  it("passes the prologue through to the handshake hash", async () => {
    const opts = (prologue?: Uint8Array) => ({
      initiator: new NoiseHandshake({
        initiator: true,
        staticKeyPair: { privateKey: INIT_STATIC_PRIV },
        identity: { seed: SEED_A },
        ephemeralKeyPair: { privateKey: INIT_EPH_PRIV },
        prologue,
      }),
      responder: new NoiseHandshake({
        initiator: false,
        staticKeyPair: { privateKey: RESP_STATIC_PRIV },
        identity: { seed: SEED_B },
        ephemeralKeyPair: { privateKey: RESP_EPH_PRIV },
        prologue,
      }),
    });
    // Deterministic runs (pinned ephemerals): the only difference is the
    // prologue, so differing handshake hashes prove it is mixed in.
    const plain = opts(undefined);
    await runDriverHandshake(plain.initiator, plain.responder);
    const hashPlain = toHex((await plain.initiator.finish()).handshakeHash);
    const withPrologue = opts(PROLOGUE);
    await runDriverHandshake(withPrologue.initiator, withPrologue.responder);
    const hashWith = toHex((await withPrologue.initiator.finish()).handshakeHash);
    expect(hashPlain).not.toBe(hashWith);
  });

  it("rejects a non-empty flight-1 payload (responder)", async () => {
    const responder = new NoiseHandshake({
      initiator: false,
      staticKeyPair: { privateKey: RESP_STATIC_PRIV },
      identity: { seed: SEED_B },
    });
    const rawInitiator = new NoiseXX({
      initiator: true,
      staticKeyPair: { privateKey: INIT_STATIC_PRIV },
    });
    const f1 = encodeLengthPrefixed(rawInitiator.writeMessage(Uint8Array.of(0x01)));
    await expect(responder.readMessage(f1)).rejects.toThrow(/flight-1/);
  });

  it("rejects a tampered flight-2 frame (AEAD)", async () => {
    const { initiator, responder } = await makeDriverPair();
    const f1 = await initiator.writeMessage();
    await responder.readMessage(f1);
    const f2 = await responder.writeMessage();
    const tampered = new Uint8Array(f2);
    tampered[40] ^= 0x01; // inside the sealed `s` token (offset 2 + 32 e)
    await expect(initiator.readMessage(tampered)).rejects.toThrow();
  });

  it("fails finish() when the remote payload carries no identity", async () => {
    const initiator = new NoiseHandshake({
      initiator: true,
      staticKeyPair: { privateKey: INIT_STATIC_PRIV },
      identity: { seed: SEED_A },
    });
    const f1 = await initiator.writeMessage();
    const rawResponder = new NoiseXX({
      initiator: false,
      staticKeyPair: { privateKey: RESP_STATIC_PRIV },
    });
    rawResponder.readMessage(decodeLengthPrefixed(f1));
    const f2 = encodeLengthPrefixed(rawResponder.writeMessage(EMPTY));
    await initiator.readMessage(f2);
    await initiator.writeMessage(); // flight 3 — verification happens at finish
    await expect(initiator.finish()).rejects.toThrow(/identity/);
  });

  it("fails finish() when the remote identity signature does not verify", async () => {
    const SEED_C = fromHex(
      "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
    );
    const initiator = new NoiseHandshake({
      initiator: true,
      staticKeyPair: { privateKey: INIT_STATIC_PRIV },
      identity: { seed: SEED_A },
    });
    const f1 = await initiator.writeMessage();
    const rawResponder = new NoiseXX({
      initiator: false,
      staticKeyPair: { privateKey: RESP_STATIC_PRIV },
    });
    rawResponder.readMessage(decodeLengthPrefixed(f1));
    // Forged flight 2: claims identity A, but the signature over the
    // responder static key is made by identity C → must not verify.
    const forged = encodeHandshakePayload({
      identityKey: encodeIdentityPublicKey(getPublicKeyEd25519(SEED_A)),
      identitySig: await signNoiseStaticKey(SEED_C, getPublicKey(RESP_STATIC_PRIV)),
    });
    const f2 = encodeLengthPrefixed(rawResponder.writeMessage(forged));
    await initiator.readMessage(f2);
    await initiator.writeMessage(); // flight 3 — verification happens at finish
    await expect(initiator.finish()).rejects.toThrow(/signature/);
  });

  it("createNoiseStaticKeypair yields fresh usable X25519 keypairs", () => {
    const a = createNoiseStaticKeypair();
    const b = createNoiseStaticKeypair();
    expect(a.privateKey).toHaveLength(32);
    expect(toHex(a.privateKey)).not.toBe(toHex(b.privateKey));
    const pub = getPublicKey(a.privateKey);
    expect(pub).toHaveLength(32);
  });
});
