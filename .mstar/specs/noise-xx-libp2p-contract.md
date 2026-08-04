# Noise XX + libp2p framing contract (frozen)

**Status:** Frozen normative spec — the wire contract for the Noise XX mesh
transport in the `@42ch/spoke-connect` TypeScript client (`./noise` subpath)
and the Rust reference `spoke-connect`. Grounded in rust-libp2p
`libp2p-noise` 0.46.x (pulled by workspace `libp2p = 0.56.0`) as used by
`crates/spoke-connect` (`noise::Config::new` in `src/node.rs`).

## 1. Interop target

| Item | Value |
|------|-------|
| Noise protocol name | `Noise_XX_25519_ChaChaPoly_SHA256` |
| DH | X25519 (32-byte keys) |
| AEAD | ChaCha20-Poly1305 (IETF / RFC 8439) |
| Hash / KDF | SHA-256 + HKDF-SHA256 (Noise `HKDF` with hash = SHA-256) |
| Multistream protocol id | `/noise` (negotiated **before** Noise bytes on a full libp2p stack) |
| Prologue (default) | empty (`[]`) — both sides MUST use the same prologue |
| Reference construction | `libp2p::noise::Config::new(&identity_keypair)` |

`crates/spoke-connect` composes `noise::Config::new` + yamux + tcp + identify.
The pure-TS `./noise` subpath matches the **Noise layer** of that stack
(handshake + length-prefixed AEAD frames). Yamux, multistream-select host
wiring, dial/listen, and identify are product-host concerns outside this
contract; the golden transcript fixture records **post-multistream Noise
bytes** (see §6).

## 2. Noise XX message pattern

Standard Noise XX (three flight messages). Roles: **I** = initiator
(dialer), **R** = responder (listener).

```
-> e
<- e, ee, s, es
-> s, se
```

| Flight | Direction | Tokens | Cleartext payload |
|--------|-----------|--------|-------------------|
| 1 | I → R | `e` | **empty** (`NoiseHandshakePayload` with all fields default/empty) |
| 2 | R → I | `e, ee, s, es` | **identity payload** (see §4) |
| 3 | I → R | `s, se` | **identity payload** (see §4) |

### 2.1 Static DH keys (`s` tokens) — REQUIRED

libp2p Noise XX **always** carries a local static X25519 keypair in the `s`
tokens. The static key is **ephemeral-per-process** by default in rust-libp2p
(`Keypair::new()` then `into_authentic`), **not** the long-term Ed25519
identity key itself.

- Each side generates (or supplies) a 32-byte X25519 static secret.
- Public static key is transmitted encrypted under the XX pattern as the `s`
  token (Noise AEAD ciphertext of the 32-byte public key + tag).
- After XX completes, `get_remote_static()` is always present; missing remote
  static is a fatal handshake error.

**Ruling:** empty/"null" `s` tokens are **not** used. Empty static keys are
not wire-compatible with rust-libp2p Noise and will fail the golden
transcript / live mesh path.

### 2.2 Handshake hash and split

After flight 3, both parties:

1. Hold identical handshake hash `h` (32 bytes, SHA-256 chaining).
2. Call Noise `Split` → two independent CipherStates (`cs_i`, `cs_r`):
   - Initiator encrypts with the first, decrypts with the second.
   - Responder encrypts with the second, decrypts with the first.
3. Transition from `HandshakeState` to `TransportState` (no further handshake
   messages).

Transport nonces start at 0 and increment per AEAD seal/open on that
CipherState (Noise standard; 64-bit little-endian nonce for ChaChaPoly).

## 3. Length-prefix framing (all Noise messages)

Every Noise handshake message and every post-handshake transport frame on the
wire is a **u16 big-endian length prefix** followed by that many ciphertext
bytes:

```
| len_be: u16 | ciphertext: len bytes |
```

| Constant | Value | Source |
|----------|------:|--------|
| Max Noise message (ciphertext) length | **65535** | Noise / libp2p-noise `MAX_NOISE_MSG_LEN` |
| Extra encrypt headroom | 1024 | libp2p-noise `EXTRA_ENCRYPT_SPACE` |
| Max cleartext payload per transport frame | **65535 − 1024 = 64511** | `MAX_FRAME_LEN` |

Rules:

- `len` is the ciphertext byte count only (not including the 2-byte prefix).
- Handshake and transport use the same length-prefix codec.
- A reader that has fewer than `2 + len` bytes buffered MUST wait (no partial
  AEAD open).
- Cleartext larger than `MAX_FRAME_LEN` MUST be split across multiple
  transport frames (libp2p-noise write path buffers up to `MAX_FRAME_LEN`
  then seals).

### 3.1 AEAD frame contents

| Phase | Seal input (cleartext) | Cipher |
|-------|------------------------|--------|
| Handshake flight | Protobuf-encoded `NoiseHandshakePayload` (may be zero-length encoding of default message) | Handshake CipherState (`write_message` / `read_message`) |
| Post-handshake | Raw application bytes (after Noise, yamux sees a byte stream; SPOKE envelopes are a higher layer) | Transport CipherState |

Post-handshake AEAD:

- Key = split transport key for that direction (32 bytes).
- Nonce = per-direction counter (Noise ChaChaPoly convention).
- AAD = empty (Noise transport mode default).
- Output ciphertext length = cleartext length + 16 (Poly1305 tag).

## 4. libp2p identity payload (static-key ↔ long-term identity binding)

### 4.1 Ruling (architect)

**Integrator-grade mesh interop with `crates/spoke-connect` REQUIRES the
libp2p Noise identity payload.**

Layering (both layers stay):

| Layer | What authenticates | Mechanism |
|-------|--------------------|-----------|
| **Transport (Noise)** | libp2p `PeerId` of the remote | XX static DH + payload signature over static key (§4.2–4.3) |
| **Session (SPOKE)** | connect hello / allowlist / capability-token | JCS + Ed25519 `noise-peerid` hello above the secure stream ([`spoke-connect.md`](./spoke-connect.md) §Identity binding) |

"Secure-transport-only XX with empty identity payloads + SPOKE hello alone"
is **insufficient** for rust-libp2p Noise: the reference calls
`finish()` which requires a verifiable `identity_key` + `identity_sig` and
returns `BadSignature` / `AuthenticationFailed` otherwise. That path is how
the Noise-authenticated `PeerId` is established before identify/hello.

The WebSocket language-native client has no Noise layer; identity is
hello-only. The mesh Noise transport is a different adapter with an extra
binding step — it does **not** replace the SPOKE hello.

### 4.2 Domain-separated static-key signature

```
STATIC_KEY_DOMAIN = "noise-libp2p-static-key:"   # ASCII, no NUL
signed_preimage  = STATIC_KEY_DOMAIN_bytes || static_public_x25519_bytes
identity_sig     = Ed25519.Sign(identity_private, signed_preimage)
```

- `static_public_x25519_bytes`: 32-byte X25519 public key (same bytes carried
  in the Noise `s` token plaintext before AEAD).
- `identity_private` / `identity_public`: the long-term libp2p identity key
  (protocol_version 1: Ed25519). Same key that derives SPOKE `peer_id`.
- Verify: `Ed25519.Verify(identity_public, signed_preimage, identity_sig)`.

### 4.3 Protobuf `NoiseHandshakePayload`

Canonical schema (libp2p-noise `payload.proto`):

```protobuf
message NoiseExtensions {
    repeated bytes webtransport_certhashes = 1;
    repeated string stream_muxers = 2;
}

message NoiseHandshakePayload {
    bytes identity_key = 1;              // libp2p identity PublicKey protobuf
    bytes identity_sig = 2;              // Ed25519 sig over static-key preimage
    optional NoiseExtensions extensions = 4;
}
```

| Field | Flight 1 (I→R) | Flight 2 (R→I) | Flight 3 (I→R) |
|-------|----------------|----------------|----------------|
| `identity_key` | empty | required | required |
| `identity_sig` | empty | required | required |
| `extensions` | omit | optional (WebTransport certhashes if configured) | optional |

`identity_key` encoding: libp2p `PublicKey` protobuf
(`key_type = Ed25519` + 32-byte raw public key) — the same protobuf envelope
used in identity multihash / `peer_id` derivation
([`spoke-connect.md`](./spoke-connect.md) §Identity binding).

After both identity payloads are received and XX completes:

1. Decode remote `identity_key` → Ed25519 public key → `PeerId`.
2. Verify `identity_sig` over `STATIC_KEY_DOMAIN || remote_static_public`.
3. On failure: abort (map to authentication failure); do not open transport.

The WebTransport certhash extension is **out of scope** (no WebTransport
path). Implementers MAY parse and ignore unknown extension fields; they MUST
NOT require extensions for the golden path.

### 4.4 Relationship to SPOKE `peer_id`

The Noise-authenticated `PeerId` (from payload `identity_key`) is the
transport identity that SPOKE session-core compares against hello
`peer_id` when an authenticated transport is present
([`spoke-connect.md`](./spoke-connect.md) §Binding rules —
"Authenticated transport"). The `./noise` subpath exposes the remote
identity public key / `peer_id` to the transport adapter after handshake so
the existing hello gate can enforce equality.

## 5. Handshake sequence (libp2p roles)

Matches `libp2p_noise::Config` upgrade futures:

**Initiator (outbound / dialer):**

1. `send` flight 1 — empty payload.
2. `recv` flight 2 — parse identity payload; store remote identity + sig.
3. `send` flight 3 — local identity payload.
4. `finish` — verify remote sig; split transport keys.

**Responder (inbound / listener):**

1. `recv` flight 1 — expect empty payload (non-empty → invalid data).
2. `send` flight 2 — local identity payload.
3. `recv` flight 3 — parse identity payload.
4. `finish` — verify remote sig; split transport keys.

## 6. Multistream-select boundary

| Concern | Ownership |
|---------|-----------|
| `/noise` protocol id string | Documented constant; exposed in the public `./noise` exports |
| Full multistream-select 1.0 negotiation over TCP | **Optional helper** — product hosts may negotiate before handing the stream to `NoiseXX` |
| Yamux / mplex | **Out of scope** (later capability) |
| Identify protocol | **Out of scope** (Rust reference / product) |

The golden transcript fixture (`tests/noise/fixtures/noise-xx-golden.json`)
records **pure Noise frames after multistream** — the `/noise` negotiation
lives outside the Noise messages.

## 7. Crypto primitive sources (TS)

| Primitive | Source | Notes |
|-----------|--------|-------|
| X25519 DH | `@noble/curves` (`@noble/curves/x25519`) | Subpath-only dependency — never resolved by the default bundles |
| ChaCha20-Poly1305 | `@noble/ciphers` (`chacha20poly1305`) | Subpath-only dependency — never resolved by the default bundles |
| SHA-256 / HKDF | `@noble/hashes` (existing package dep) | Import only from `src/noise/**` for Noise KDF paths |
| Ed25519 (static-key sig + identity) | Reuse package `@noble/ed25519` / WebCrypto helpers already used for hello | Same long-term key as SPOKE identity |

Noise imports stay inside `src/noise/**`; `@noble/ciphers` and
`@noble/curves` are never resolved by the default `.` / `./node` bundles.

## 8. Explicit non-goals (this contract)

- IK / IX patterns (libp2p supports XX only for the guaranteed path).
- Non-empty prologue variants (unless both sides configure the same bytes;
  default golden path is empty prologue).
- Replacing SPOKE hello with Noise payload identity.
- Widening session-core parity to include Noise (Noise is
  **transport-adapter-owned**).
- DHT, yamux, mDNS, WebTransport certhash enforcement.

## 9. Verification gates

The implementation satisfies all of the following:

- RFC vectors: X25519, HKDF-SHA256, ChaCha20-Poly1305.
- Noise XX test vector: identical `h` and split keys both sides.
- Length-prefix encode/decode + max-frame split behavior.
- Payload protobuf round-trip; static-key domain signature verify.
- Golden rust-libp2p initiator transcript → TS responder derives same
  transport keys and decrypts one post-handshake frame.
- Default `@42ch/spoke-connect` entry does not import `@noble/ciphers` or
  `@noble/curves`.
