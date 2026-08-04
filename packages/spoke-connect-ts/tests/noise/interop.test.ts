/**
 * rust-libp2p Noise XX golden-transcript interop gate (Task 4,
 * connect-ts-noise-stack) — the cross-language correctness gate for the
 * pure-TS Noise stack.
 *
 * The fixture `noise-xx-golden.json` is a GENUINE rust-libp2p recording:
 * `Noise_XX_25519_ChaChaPoly_SHA256` driven with the exact engine behind
 * `libp2p-noise` 0.46.1 (libp2p 0.56.0) — `snow` 0.9.6 with the same
 * builder parameters `libp2p_noise::Config` composes, pinned static +
 * ephemeral + identity keys — via the dev-only recorder
 * `crates/spoke-connect/examples/noise_recorder.rs`. Bytes are pure Noise
 * frames after multistream (the `/noise` negotiation lives outside the
 * Noise messages; frozen contract §6).
 *
 * This test plays the RESPONDER: it reconstructs the TS `NoiseHandshake`
 * with the pinned responder keys, feeds it the recorded initiator flights,
 * asserts the flight it produces matches the recorded responder flight
 * byte-for-byte, and proves the split transport keys are identical by
 * decrypting the recorded initiator→responder frame and re-sealing the
 * recorded responder→initiator frame to the same bytes.
 */
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { NoiseHandshake } from "../../src/noise/framing.js";
import {
  decodeHandshakePayload,
  encodeIdentityPublicKey,
} from "../../src/noise/payload.js";
import { fromHex, toHex } from "../hex.js";

interface GoldenFixture {
  protocol: string;
  prologue: string;
  framing: string;
  source: string;
  roles: { initiator: string; responder: string };
  keys: {
    initiator: Record<string, string>;
    responder: Record<string, string>;
  };
  flights: { flight1: string; flight2: string; flight3: string };
  payloads: { flight1: string; flight2: string; flight3: string };
  handshake: { handshakeHash: string };
  transport: {
    initiatorToResponder: { frame: string; plaintext: string; note: string };
    responderToInitiator: { frame: string; plaintext: string; note: string };
  };
}

const fixture: GoldenFixture = JSON.parse(
  readFileSync(
    new URL("./fixtures/noise-xx-golden.json", import.meta.url),
    "utf8",
  ),
);

describe("rust-libp2p Noise XX golden transcript (interop)", () => {
  it("records the contract wire sizes (u16-BE prefix, empty flight-1 payload)", () => {
    expect(fixture.protocol).toBe("Noise_XX_25519_ChaChaPoly_SHA256");
    expect(fixture.prologue).toBe("");
    // flight1 = 2 + 32 (e.pub, payload rides in the clear), flight2 =
    // 2 + 32 + 48 + 104 + 16 (e, ee, s, es + identity payload + tag),
    // flight3 = 2 + 48 + 104 + 16 (s, se + identity payload + tag).
    expect(fromHex(fixture.flights.flight1)).toHaveLength(2 + 32);
    expect(fromHex(fixture.flights.flight2)).toHaveLength(2 + 32 + 48 + 104 + 16);
    expect(fromHex(fixture.flights.flight3)).toHaveLength(2 + 48 + 104 + 16);
    // Flight 1 rides in the clear with an empty payload (§5).
    expect(fixture.payloads.flight1).toBe("");
    // Flights 2–3 carry the 104-byte NoiseHandshakePayload.
    expect(fixture.payloads.flight2).toHaveLength(104 * 2);
    expect(fixture.payloads.flight3).toHaveLength(104 * 2);
  });

  it("decodes the recorded rust-libp2p identity payloads with the TS protobuf codec", () => {
    for (const [flight, side] of [
      ["flight2", "responder"],
      ["flight3", "initiator"],
    ] as const) {
      const payload = decodeHandshakePayload(fromHex(fixture.payloads[flight]));
      expect(payload.identityKey).toBeDefined();
      expect(payload.identitySig).toBeDefined();
      expect(payload.identitySig).toHaveLength(64);
      expect(toHex(payload.identityKey!)).toBe(
        toHex(encodeIdentityPublicKey(fromHex(fixture.keys[side].identityPublic))),
      );
    }
  });

  it(
    "completes XX as responder against the recorded initiator transcript " +
      "with byte-identical flights and identical transport keys",
    async () => {
      const k = fixture.keys;
      const responder = new NoiseHandshake({
        initiator: false,
        staticKeyPair: { privateKey: fromHex(k.responder.staticPrivate) },
        identity: { seed: fromHex(k.responder.identitySeed) },
        ephemeralKeyPair: { privateKey: fromHex(k.responder.ephemeralPrivate) },
      });

      // Flight 1 (initiator → responder): read; must carry an empty payload.
      await responder.readMessage(fromHex(fixture.flights.flight1));

      // Flight 2 (responder → initiator): byte-for-byte match with the
      // recorded rust-libp2p responder flight.
      const flight2 = await responder.writeMessage();
      expect(toHex(flight2)).toBe(fixture.flights.flight2);

      // Flight 3 (initiator → responder): read + parse identity payload.
      await responder.readMessage(fromHex(fixture.flights.flight3));

      // finish() verifies the remote identity signature over the remote
      // static key and splits the transport keys.
      const result = await responder.finish();

      // Handshake hash identical to the recording.
      expect(toHex(result.handshakeHash)).toBe(fixture.handshake.handshakeHash);
      // Remote static X25519 key = the initiator's pinned static public key.
      expect(toHex(result.remoteStaticPublic)).toBe(k.initiator.staticPublic);
      // Remote identity = the initiator's pinned Ed25519 public key.
      expect(toHex(result.remoteIdentityPublicKey)).toBe(
        k.initiator.identityPublic,
      );
      expect(result.remotePeerId).toBe(k.initiator.peerId);

      // Transport keys are identical to the Rust recording: the recorded
      // initiator→responder frame (sealed under c1, nonce 0) decrypts under
      // the responder RX key…
      const opened = result.transport.readFrame(
        fromHex(fixture.transport.initiatorToResponder.frame),
      );
      expect(toHex(opened)).toBe(fixture.transport.initiatorToResponder.plaintext);

      // …and the responder TX key re-seals the recorded plaintext to the
      // recorded responder→initiator frame byte-for-byte.
      const resealed = result.transport.writeFrame(
        fromHex(fixture.transport.responderToInitiator.plaintext),
      );
      expect(toHex(resealed)).toBe(fixture.transport.responderToInitiator.frame);
    },
  );
});
