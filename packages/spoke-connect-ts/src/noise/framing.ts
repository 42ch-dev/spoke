/**
 * libp2p Noise wire framing — the u16-BE length-prefix codec, the
 * `NoiseHandshake` driver that wraps `NoiseXX` with framing + identity
 * payload, and the post-split `NoiseTransport` AEAD frame codec (frozen
 * contract `.mstar/specs/noise-xx-libp2p-contract.md` §3, §5).
 *
 * Wire shape of every Noise message (handshake flights and post-handshake
 * transport frames alike):
 *
 *     | len_be: u16 | ciphertext: len bytes |
 *
 * `len` counts the ciphertext bytes only. Handshake messages and sealed
 * transport frames are capped at `MAX_NOISE_MSG_LEN` (65535); cleartext
 * larger than `MAX_FRAME_LEN` (64511 = 65535 − 1024 headroom) is split
 * across multiple frames. Post-handshake AEAD: AAD = empty, per-direction
 * nonce counters from 0 (contract §3 / §3.1).
 *
 * The `NoiseHandshake` driver owns the libp2p role sequence (contract §5):
 *
 *   initiator:  write flight 1 (empty payload) → read flight 2 →
 *               write flight 3 (identity payload) → finish (verify + split)
 *   responder:  read flight 1 (must be empty) → write flight 2 (identity
 *               payload) → read flight 3 → finish (verify + split)
 *
 * Imports stay inside `src/noise/**` plus shared src-root helpers already
 * in the default bundle (plan "Global Constraints").
 */
import { x25519 } from "@noble/curves/ed25519.js";
import { concatBytes } from "@noble/hashes/utils.js";

import { getPublicKeyEd25519 } from "../crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../identity.js";
import { decrypt, encrypt, getPublicKey } from "./crypto.js";
import {
  decodeHandshakePayload,
  decodeIdentityPublicKey,
  encodeHandshakePayload,
  encodeIdentityPublicKey,
  signNoiseStaticKey,
  verifyNoiseStaticKey,
} from "./payload.js";
import { NoiseXX, type NoiseKeyPair } from "./xx.js";

/** Multistream protocol id for the Noise layer (contract §1, §6). */
export const NOISE_PROTOCOL_ID = "/noise";

/** Max length of a Noise message (ciphertext) — libp2p-noise
 *  `MAX_NOISE_MSG_LEN`; also the u16 prefix maximum. */
export const MAX_NOISE_MSG_LEN = 65535;
/** Extra encrypt headroom reserved per frame — libp2p-noise
 *  `EXTRA_ENCRYPT_SPACE`. */
export const EXTRA_ENCRYPT_SPACE = 1024;
/** Max cleartext per post-handshake transport frame. */
export const MAX_FRAME_LEN = MAX_NOISE_MSG_LEN - EXTRA_ENCRYPT_SPACE; // 64511

function readU16BE(bytes: Uint8Array, offset: number): number {
  return new DataView(bytes.buffer, bytes.byteOffset + offset, 2).getUint16(0, false);
}

// ── §3 u16-BE length-prefix codec ───────────────────────────────────────────

/**
 * Prefix `payload` with its u16 big-endian length (ciphertext byte count
 * only, not including the prefix). Throws above `MAX_NOISE_MSG_LEN`.
 */
export function encodeLengthPrefixed(payload: Uint8Array): Uint8Array {
  if (payload.length > MAX_NOISE_MSG_LEN) {
    throw new Error(
      `Noise message too large: ${payload.length} > ${MAX_NOISE_MSG_LEN}`,
    );
  }
  const frame = new Uint8Array(2 + payload.length);
  frame[0] = payload.length >> 8;
  frame[1] = payload.length & 0xff;
  frame.set(payload, 2);
  return frame;
}

/**
 * Strip the u16-BE length prefix of exactly one frame. Throws when the
 * input is not a complete frame (truncated header/payload) or carries
 * trailing bytes — callers own stream buffering; partial frames must not
 * be opened (contract §3: "MUST wait").
 */
export function decodeLengthPrefixed(frame: Uint8Array): Uint8Array {
  if (frame.length < 2) {
    throw new Error("Noise frame truncated: missing u16 length prefix");
  }
  const length = readU16BE(frame, 0);
  if (frame.length < 2 + length) {
    throw new Error(
      `Noise frame truncated: declared ${length} bytes, have ${frame.length - 2}`,
    );
  }
  if (frame.length > 2 + length) {
    throw new Error(
      `Noise frame has ${frame.length - 2 - length} trailing bytes after its payload`,
    );
  }
  return frame.slice(2, 2 + length);
}

/**
 * Split a buffer of concatenated length-prefixed frames into the
 * individual frames (complement of multi-frame `writeFrame` output).
 */
export function splitLengthPrefixedFrames(bytes: Uint8Array): Uint8Array[] {
  const frames: Uint8Array[] = [];
  let offset = 0;
  while (offset < bytes.length) {
    if (bytes.length - offset < 2) {
      throw new Error("Noise frame truncated: missing u16 length prefix");
    }
    const length = readU16BE(bytes, offset);
    const end = offset + 2 + length;
    if (end > bytes.length) {
      throw new Error(
        `Noise frame truncated: declared ${length} bytes at offset ${offset}`,
      );
    }
    frames.push(bytes.slice(offset, end));
    offset = end;
  }
  return frames;
}

/**
 * A fresh random X25519 static keypair (contract §2.1 — static keys are
 * ephemeral-per-process, not the long-term Ed25519 identity).
 */
export function createNoiseStaticKeypair(): NoiseKeyPair {
  return { privateKey: x25519.utils.randomSecretKey() };
}

// ── §3.1 NoiseTransport (post-split AEAD frames) ────────────────────────────

/** Post-split transport keys (from `NoiseXXResult` / `NoiseHandshakeResult`). */
export interface NoiseTransportKeys {
  /** Key sealing outgoing frames (initiator: first Split output). */
  tx: Uint8Array;
  /** Key opening incoming frames (initiator: second Split output). */
  rx: Uint8Array;
}

/**
 * Post-handshake transport: seals cleartext under the split key of the
 * direction with per-direction nonce counters from 0, then length-prefixes
 * (AAD = empty — Noise transport mode default). Cleartext above
 * `MAX_FRAME_LEN` is split across multiple frames (contract §3).
 */
export class NoiseTransport {
  private readonly tx: Uint8Array;
  private readonly rx: Uint8Array;
  // simplify: JS-number nonce counters; Noise rekeying at 2^64−1 is out of
  // scope (a direction would need 2^53+ frames to lose precision — a
  // transport-adapter concern, mirroring rust only in the realistic range).
  private txNonce = 0;
  private rxNonce = 0;

  constructor(keys: NoiseTransportKeys) {
    this.tx = keys.tx.slice();
    this.rx = keys.rx.slice();
  }

  /**
   * Seal `plaintext` into one or more length-prefixed frames (splits at
   * `MAX_FRAME_LEN`). Empty input produces no bytes.
   */
  writeFrame(plaintext: Uint8Array): Uint8Array {
    const frames: Uint8Array[] = [];
    let offset = 0;
    while (offset < plaintext.length) {
      const chunk = plaintext.slice(offset, offset + MAX_FRAME_LEN);
      offset += chunk.length;
      const sealed = encrypt(this.tx, this.txNonce++, new Uint8Array(0), chunk);
      frames.push(encodeLengthPrefixed(sealed));
    }
    return concatBytes(...frames);
  }

  /**
   * Open exactly one length-prefixed frame. Throws on truncation and on
   * AEAD authentication failure (tampered frame / wrong key).
   */
  readFrame(frame: Uint8Array): Uint8Array {
    const sealed = decodeLengthPrefixed(frame);
    const opened = decrypt(this.rx, this.rxNonce++, new Uint8Array(0), sealed);
    if (opened === null) {
      throw new Error(
        "NoiseTransport: AEAD authentication failed while opening a frame",
      );
    }
    return opened;
  }
}

// ── §5 NoiseHandshake driver ────────────────────────────────────────────────

/** The peer's long-term Ed25519 identity (same key as the SPOKE `peer_id`). */
export interface NoiseIdentity {
  /** 32-byte Ed25519 seed. */
  seed: Uint8Array;
}

export interface NoiseHandshakeOptions {
  /** `true` = initiator (dialer), `false` = responder (listener). */
  initiator: boolean;
  /** Local static X25519 keypair carried in the `s` tokens (§2.1). */
  staticKeyPair: NoiseKeyPair;
  /** Long-term identity signing the static key (§4). */
  identity: NoiseIdentity;
  /** Prologue; MUST match the peer. Default empty. */
  prologue?: Uint8Array;
  /** Fixed ephemeral keypair — test vectors / golden recording only. */
  ephemeralKeyPair?: NoiseKeyPair;
}

/** Result of a completed, verified handshake (contract §4.4). */
export interface NoiseHandshakeResult {
  /** Post-split frame codec (AAD = empty, per-direction nonces 0+). */
  transport: NoiseTransport;
  /** Final handshake hash `h` — identical on both parties. */
  handshakeHash: Uint8Array;
  /** Remote static X25519 public key (from the `s` tokens). */
  remoteStaticPublic: Uint8Array;
  /** Remote Ed25519 public key from the payload `identity_key`. */
  remoteIdentityPublicKey: Uint8Array;
  /** SPOKE `peer_id` of the remote identity (identity multihash). */
  remotePeerId: string;
}

/**
 * Full libp2p Noise XX exchange: frames `NoiseXX` flights with the u16-BE
 * prefix, carries the `NoiseHandshakePayload` on flights 2–3 (empty on
 * flight 1), and verifies the remote identity signature over the remote
 * static key before exposing the transport (contract §5).
 */
export class NoiseHandshake {
  private readonly initiator: boolean;
  private readonly xx: NoiseXX;
  private readonly identitySeed: Uint8Array;
  private readonly staticPublic: Uint8Array;
  /** Flights processed by this side (0..3). */
  private phase = 0;
  /** Remote `NoiseHandshakePayload` from the last read flight. */
  private remotePayload: ReturnType<typeof decodeHandshakePayload> | null = null;

  constructor(options: NoiseHandshakeOptions) {
    this.initiator = options.initiator;
    this.identitySeed = options.identity.seed.slice();
    this.staticPublic = getPublicKey(options.staticKeyPair.privateKey);
    this.xx = new NoiseXX({
      initiator: options.initiator,
      staticKeyPair: options.staticKeyPair,
      prologue: options.prologue,
      ephemeralKeyPair: options.ephemeralKeyPair,
    });
  }

  /**
   * Encode, sign (flights 2–3) and frame the next handshake flight:
   * initiator flights 1 and 3, responder flight 2.
   */
  async writeMessage(): Promise<Uint8Array> {
    const isWritePhase =
      (this.initiator && (this.phase === 0 || this.phase === 2)) ||
      (!this.initiator && this.phase === 1);
    if (!isWritePhase) {
      throw new Error(
        this.phase >= 3
          ? "NoiseHandshake: handshake already finished — no further writes"
          : "NoiseHandshake: out of turn — this role must read the next flight",
      );
    }
    const payload =
      this.phase === 0 ? new Uint8Array(0) : await this.identityPayload();
    const message = this.xx.writeMessage(payload);
    this.phase += 1;
    return encodeLengthPrefixed(message);
  }

  /**
   * Read, unframe and decode the next handshake flight. The responder
   * rejects a non-empty flight-1 payload (invalid data, contract §5).
   */
  async readMessage(frame: Uint8Array): Promise<void> {
    const payload = this.xx.readMessage(decodeLengthPrefixed(frame));
    if (!this.initiator && this.phase === 0 && payload.length > 0) {
      throw new Error(
        "NoiseHandshake: non-empty flight-1 payload from the initiator",
      );
    }
    this.remotePayload = decodeHandshakePayload(payload);
    this.phase += 1;
  }

  /**
   * Verify the remote identity signature over the remote static key and
   * split the transport keys. Throws on any authentication failure — the
   * transport is not exposed unless the binding verifies (contract §4.3
   * step 3, §5).
   */
  async finish(): Promise<NoiseHandshakeResult> {
    const split = this.xx.finish();
    const payload = this.remotePayload;
    if (
      payload === null ||
      payload.identityKey === undefined ||
      payload.identitySig === undefined
    ) {
      throw new Error(
        "NoiseHandshake: remote identity payload is missing identity_key or identity_sig",
      );
    }
    const identityPublic = decodeIdentityPublicKey(payload.identityKey);
    if (identityPublic === undefined) {
      throw new Error(
        "NoiseHandshake: remote identity_key is not an Ed25519 PublicKey",
      );
    }
    const valid = await verifyNoiseStaticKey(
      identityPublic,
      split.remoteStaticPublic,
      payload.identitySig,
    );
    if (!valid) {
      throw new Error(
        "NoiseHandshake: remote identity signature over the static key failed verification",
      );
    }
    return {
      transport: new NoiseTransport({ tx: split.tx, rx: split.rx }),
      handshakeHash: split.handshakeHash,
      remoteStaticPublic: split.remoteStaticPublic,
      remoteIdentityPublicKey: identityPublic,
      remotePeerId: derivePeerIdFromEd25519Pubkey(identityPublic),
    };
  }

  /** The `NoiseHandshakePayload` for flights 2–3 (contract §4.3). */
  private async identityPayload(): Promise<Uint8Array> {
    const identityPublic = getPublicKeyEd25519(this.identitySeed);
    const identitySig = await signNoiseStaticKey(
      this.identitySeed,
      this.staticPublic,
    );
    return encodeHandshakePayload({
      identityKey: encodeIdentityPublicKey(identityPublic),
      identitySig,
    });
  }
}
