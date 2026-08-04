/**
 * Noise XX handshake state machine — `Noise_XX_25519_ChaChaPoly_SHA256`
 * (frozen contract `.mstar/specs/noise-xx-libp2p-contract.md` §2; Noise
 * spec §5.2–§5.3).
 *
 * Message pattern (roles: I = initiator/dialer, R = responder/listener):
 *
 *     -> e            flight 1 (I → R, payload in the clear — k is empty)
 *     <- e, ee, s, es flight 2 (R → I)
 *     -> s, se        flight 3 (I → R)
 *
 * Token semantics follow the current Noise spec exactly:
 *   - `e` in a non-PSK handshake does MixHash(e.public_key) only — the
 *     MixKey(e.public_key) step is a PSK-only rule. This is what makes the
 *     flight-1 payload ride in the clear (verified against the canonical
 *     cacophony vector and snow 0.10, the crate behind rust-libp2p noise).
 *   - `s` is `EncryptAndHash(s.public_key)` (AAD = handshake hash).
 *   - `ee` / `es` / `se` are `MixKey(DH(...))` with the spec's
 *     initiator/responder swap for `es` / `se`.
 *   - AEAD nonces continue across flights (no reset between messages);
 *     every MixKey resets the nonce to 0.
 *   - `Split()` = `HKDF(ck, empty, 2×32)`: the initiator encrypts with the
 *     first output and decrypts with the second; the responder mirrors.
 *
 * The static keys are REAL X25519 keypairs (contract §2.1 — empty `s` is
 * not wire-compatible with rust-libp2p). The handshake payload is opaque
 * bytes at this layer: the libp2p `NoiseHandshakePayload` encoding, signing
 * and verification belong to Task 3 (framing/payload). A failed read aborts
 * the handshake (spec: "the handshake has failed and the HandshakeState is
 * deleted") — no rollback/retry semantics here.
 *
 * Imports stay inside `src/noise/**` (default-bundle isolation rule, plan
 * "Global Constraints").
 */
import { x25519 } from "@noble/curves/ed25519.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { concatBytes } from "@noble/hashes/utils.js";

import { decrypt, dh, encrypt, getPublicKey, hkdf } from "./crypto.js";

/** Noise protocol name (initialize symmetric state from this name). */
export const NOISE_PROTOCOL_NAME = "Noise_XX_25519_ChaChaPoly_SHA256";

const HASHLEN = 32; // SHA-256
const DHLEN = 32; // X25519 public key
const TAGLEN = 16; // ChaCha20-Poly1305 tag

/** Local X25519 keypair (private key; public key is derived). */
export interface NoiseKeyPair {
  privateKey: Uint8Array;
}

export interface NoiseXXOptions {
  /** `true` = initiator (dialer), `false` = responder (listener). */
  initiator: boolean;
  /** Local static X25519 keypair carried in the `s` tokens (contract §2.1). */
  staticKeyPair: NoiseKeyPair;
  /** Prologue mixed into the handshake hash; MUST match on both sides. */
  prologue?: Uint8Array;
  /**
   * Fixed ephemeral keypair (test vectors only — cacophony pins the
   * ephemeral keys). When omitted, a fresh random ephemeral is generated
   * for every handshake.
   */
  ephemeralKeyPair?: NoiseKeyPair;
}

/** Post-split transport material (contract §2.2). */
export interface NoiseXXResult {
  /** Key for sealing our outgoing transport frames (AAD = empty, nonce 0+). */
  tx: Uint8Array;
  /** Key for opening incoming transport frames (AAD = empty, nonce 0+). */
  rx: Uint8Array;
  /** Final handshake hash `h` — identical on both parties. */
  handshakeHash: Uint8Array;
  /** Remote static X25519 public key (always present after a full XX). */
  remoteStaticPublic: Uint8Array;
}

type Token = "e" | "s" | "ee" | "es" | "se";

/** The three XX flights, in wire order (roles alternate around them). */
const MESSAGE_PATTERNS: Token[][] = [
  ["e"], // flight 1: I → R
  ["e", "ee", "s", "es"], // flight 2: R → I
  ["s", "se"], // flight 3: I → R
];

export class NoiseXX {
  private readonly initiator: boolean;

  // SymmetricState (Noise spec §5.2)
  private ck: Uint8Array;
  private h: Uint8Array;
  private k: Uint8Array | null = null;
  private nonce = 0;

  // HandshakeState (Noise spec §5.3)
  private readonly s: { privateKey: Uint8Array; publicKey: Uint8Array };
  private e: { privateKey: Uint8Array; publicKey: Uint8Array } | null = null;
  private re: Uint8Array | null = null; // remote ephemeral public key
  private rs: Uint8Array | null = null; // remote static public key

  private readonly fixedEphemeral: { privateKey: Uint8Array } | null;

  /** 0..2 — which flight the next write/read processes; 3 = finished. */
  private phase = 0;

  // Split results, computed on the final flight (Noise spec: Split() is
  // called once the message patterns are exhausted).
  private tx: Uint8Array | null = null;
  private rx: Uint8Array | null = null;

  constructor(options: NoiseXXOptions) {
    this.initiator = options.initiator;
    this.fixedEphemeral = options.ephemeralKeyPair ?? null;

    const name = new TextEncoder().encode(NOISE_PROTOCOL_NAME);
    if (name.length > HASHLEN) {
      this.h = sha256(name);
    } else {
      // protocol_name ≤ HASHLEN: h = name zero-padded (exactly HASHLEN here).
      this.h = new Uint8Array(HASHLEN);
      this.h.set(name);
    }
    this.ck = this.h.slice();
    // MixHash(prologue) — empty prologue leaves h = SHA-256(protocol_name).
    this.h = sha256(concatBytes(this.h, options.prologue ?? new Uint8Array(0)));

    this.s = {
      privateKey: options.staticKeyPair.privateKey.slice(),
      publicKey: getPublicKey(options.staticKeyPair.privateKey),
    };
  }

  // ── Symmetric operations (Noise spec §5.2) ─────────────────────────────

  private mixHash(data: Uint8Array): void {
    this.h = sha256(concatBytes(this.h, data));
  }

  private mixKey(inputKeyMaterial: Uint8Array): void {
    const { chainingKey, key } = hkdf(this.ck, inputKeyMaterial);
    this.ck = chainingKey;
    this.k = key;
    this.nonce = 0;
  }

  /** EncryptAndHash: seal under AAD = h (or copy when k is empty), then mix. */
  private encryptAndHash(plaintext: Uint8Array): Uint8Array {
    const out = this.k
      ? encrypt(this.k, this.nonce++, this.h, plaintext)
      : plaintext.slice();
    this.mixHash(out);
    return out;
  }

  /** DecryptAndHash: open under AAD = h (or copy when k is empty), then mix. */
  private decryptAndHash(ciphertext: Uint8Array): Uint8Array {
    let plaintext: Uint8Array;
    if (this.k) {
      const opened = decrypt(this.k, this.nonce++, this.h, ciphertext);
      if (opened === null) {
        throw new Error(
          "NoiseXX: AEAD authentication failed while reading a handshake message",
        );
      }
      plaintext = opened;
    } else {
      plaintext = ciphertext.slice();
    }
    this.mixHash(ciphertext);
    return plaintext;
  }

  // ── Token processing (Noise spec §5.3) ──────────────────────────────────

  private writeToken(token: Token): Uint8Array {
    switch (token) {
      case "e": {
        const privateKey = this.fixedEphemeral
          ? this.fixedEphemeral.privateKey.slice()
          : x25519.utils.randomSecretKey();
        this.e = { privateKey, publicKey: getPublicKey(privateKey) };
        const pub = this.e.publicKey;
        this.mixHash(pub);
        return pub;
      }
      case "s":
        // EncryptAndHash(s.public_key); in XX, k is always set when `s`
        // appears (flights 2–3 follow a MixKey), so this is AEAD-sealed.
        return this.encryptAndHash(this.s.publicKey);
      case "ee":
        this.mixKey(dh(this.e!.privateKey, this.re!));
        return new Uint8Array(0);
      case "es":
        this.mixKey(
          this.initiator
            ? dh(this.e!.privateKey, this.rs!)
            : dh(this.s.privateKey, this.re!),
        );
        return new Uint8Array(0);
      case "se":
        this.mixKey(
          this.initiator
            ? dh(this.s.privateKey, this.re!)
            : dh(this.e!.privateKey, this.rs!),
        );
        return new Uint8Array(0);
    }
  }

  /** Bytes a `token` consumes from an incoming message at the current state. */
  private readTokenLength(token: Token): number {
    switch (token) {
      case "e":
        return DHLEN;
      case "s":
        return this.k ? DHLEN + TAGLEN : DHLEN;
      default:
        return 0; // DH tokens consume no message bytes
    }
  }

  private readToken(token: Token, message: Uint8Array, offset: number): number {
    switch (token) {
      case "e": {
        this.re = message.slice(offset, offset + DHLEN);
        this.mixHash(this.re);
        return DHLEN;
      }
      case "s": {
        const len = this.k ? DHLEN + TAGLEN : DHLEN;
        this.rs = this.decryptAndHash(message.slice(offset, offset + len));
        return len;
      }
      case "ee":
        this.mixKey(dh(this.e!.privateKey, this.re!));
        return 0;
      case "es":
        this.mixKey(
          this.initiator
            ? dh(this.e!.privateKey, this.rs!)
            : dh(this.s.privateKey, this.re!),
        );
        return 0;
      case "se":
        this.mixKey(
          this.initiator
            ? dh(this.s.privateKey, this.re!)
            : dh(this.e!.privateKey, this.rs!),
        );
        return 0;
    }
  }

  // ── Public API ───────────────────────────────────────────────────────────

  /**
   * Write the next handshake flight with `payload` (opaque bytes; the
   * libp2p `NoiseHandshakePayload` encoding is Task 3's concern). Returns
   * the full wire message for this flight. Call in order: initiator
   * writes flights 1 and 3, responder writes flight 2.
   */
  writeMessage(payload: Uint8Array): Uint8Array {
    this.assertCanWrite();
    const pattern = MESSAGE_PATTERNS[this.phase];
    const chunks: Uint8Array[] = [];
    for (const token of pattern) {
      const out = this.writeToken(token);
      if (out.length > 0) chunks.push(out);
    }
    chunks.push(this.encryptAndHash(payload));
    this.advancePhase();
    return concatBytes(...chunks);
  }

  /**
   * Read the next handshake flight. Returns the plaintext payload (opaque
   * bytes). Throws on truncated messages and on AEAD authentication
   * failure — a failed read aborts the handshake.
   */
  readMessage(message: Uint8Array): Uint8Array {
    this.assertCanRead();
    const pattern = MESSAGE_PATTERNS[this.phase];
    let offset = 0;
    for (const token of pattern) {
      const len = this.readTokenLength(token);
      if (offset + len > message.length) {
        throw new Error("NoiseXX: truncated handshake message");
      }
      offset += this.readToken(token, message, offset);
    }
    const payload = this.decryptAndHash(message.slice(offset));
    this.advancePhase();
    return payload;
  }

  /**
   * Complete the handshake: returns the split transport keys, the final
   * handshake hash, and the remote static public key. Callable only after
   * all three flights have been processed by this party.
   */
  finish(): NoiseXXResult {
    if (this.phase !== 3) {
      throw new Error(
        "NoiseXX: finish() before the handshake completed (3 flights)",
      );
    }
    if (this.rs === null || this.tx === null || this.rx === null) {
      throw new Error("NoiseXX: finish() without remote static key");
    }
    return {
      tx: this.tx.slice(),
      rx: this.rx.slice(),
      handshakeHash: this.h.slice(),
      remoteStaticPublic: this.rs.slice(),
    };
  }

  // ── Internals ────────────────────────────────────────────────────────────

  /** Split(): HKDF(ck, empty) → (c1, c2); initiator seals with c1, opens
   *  with c2, the responder mirrors (Noise spec §5.2). */
  private split(): void {
    const { chainingKey: c1, key: c2 } = hkdf(this.ck, new Uint8Array(0));
    if (this.initiator) {
      this.tx = c1;
      this.rx = c2;
    } else {
      this.tx = c2;
      this.rx = c1;
    }
  }

  private advancePhase(): void {
    this.phase += 1;
    if (this.phase === 3) this.split();
  }

  private assertCanWrite(): void {
    const writePhase =
      (this.initiator && (this.phase === 0 || this.phase === 2)) ||
      (!this.initiator && this.phase === 1);
    if (!writePhase) {
      throw new Error(
        this.phase >= 3
          ? "NoiseXX: handshake already finished — no further writes"
          : "NoiseXX: out of turn — this role must read the next flight",
      );
    }
  }

  private assertCanRead(): void {
    const readPhase =
      (this.initiator && this.phase === 1) ||
      (!this.initiator && (this.phase === 0 || this.phase === 2));
    if (!readPhase) {
      throw new Error(
        this.phase >= 3
          ? "NoiseXX: handshake already finished — no further reads"
          : "NoiseXX: out of turn — this role must write the next flight",
      );
    }
  }
}
