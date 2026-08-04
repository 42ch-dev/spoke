//! Dev-only golden-transcript recorder for the Noise XX interop gate
//! (Task 4, connect-ts-noise-stack).
//!
//! Records a `Noise_XX_25519_ChaChaPoly_SHA256` handshake transcript using
//! the exact engine behind rust-libp2p's `libp2p-noise` 0.46.1 — the crate
//! behind `crates/spoke-connect`'s `noise::Config::new` (libp2p 0.56.0):
//! `snow` 0.9.6 with the same builder parameters `libp2p_noise::Config`
//! composes in `noise_params_into_builder` (`Builder::with_resolver(PARAMS_XX,
//! RingResolver).prologue([]).local_private_key(static_secret)`), plus
//! `fixed_ephemeral_key_for_testing_only` so the transcript is fully
//! deterministic. libp2p-noise's own crypto resolver delegates hash and
//! cipher to `snow::resolvers::RingResolver` (protocol.rs), so this recorder
//! emits byte-identical Noise messages to the real stack for the same keys;
//! the u16-BE length prefix and the `NoiseHandshakePayload` protobuf +
//! Ed25519 static-key domain signature replicate `Codec::encode` /
//! `send_identity` exactly.
//!
//! Run (nightly toolchain per repo AGENTS.md), redirecting stdout to the
//! committed TS fixture:
//!
//!     cargo +nightly run -p spoke-connect --example noise_recorder \
//!         > packages/spoke-connect-ts/tests/noise/fixtures/noise-xx-golden.json
//!
//! Output is a single JSON fixture on stdout (the full transcript). Dev-only:
//! `examples/**` is excluded from the crate tarball (`exclude` in
//! Cargo.toml) and all dependencies are dev-dependencies — nothing enters
//! the published crate or the TS package.

use libp2p_identity::Keypair as IdentityKeypair;
use snow::{
    params::{CipherChoice, DHChoice, HashChoice, NoiseParams},
    resolvers::{CryptoResolver, RingResolver},
    types::{Cipher, Dh, Hash, Random},
    Builder,
};
use x25519_dalek::{x25519, X25519_BASEPOINT_BYTES};

/// The libp2p-noise static-key signature domain (libp2p-noise `protocol.rs`).
const STATIC_KEY_DOMAIN: &[u8] = b"noise-libp2p-static-key:";
/// Noise protocol name (libp2p-noise `PARAMS_XX`).
const PROTOCOL_NAME: &str = "Noise_XX_25519_ChaChaPoly_SHA256";

/// Fixed identity seeds — the same values as the TS unit fixtures
/// (tests/noise/xx.test.ts: SEED_A = initiator, SEED_B = responder).
const INIT_IDENTITY_SEED: [u8; 32] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
    0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
    0x1d, 0x1e, 0x1f, 0x20,
];
const RESP_IDENTITY_SEED: [u8; 32] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d,
    0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b,
    0x3c, 0x3d, 0x3e, 0x3f,
];
/// Pinned X25519 secrets (test-only keys).
const INIT_STATIC: [u8; 32] = [0x11; 32];
const INIT_EPHEMERAL: [u8; 32] = [0x22; 32];
const RESP_STATIC: [u8; 32] = [0x33; 32];
const RESP_EPHEMERAL: [u8; 32] = [0x44; 32];

/// Post-handshake transport plaintext (ASCII).
const TRANSPORT_PLAINTEXT: &[u8] = b"spoke-over-noise";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── Minimal protobuf encoding (libp2p-noise `payload.proto` — field order
//    matches quick-protobuf `MessageWrite`: identity_key=1, identity_sig=2) ──

fn varint(v: u64, out: &mut Vec<u8>) {
    let mut v = v;
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

fn field_bytes(num: u64, data: &[u8], out: &mut Vec<u8>) {
    varint((num << 3) | 2, out);
    varint(data.len() as u64, out);
    out.extend_from_slice(data);
}

/// `NoiseHandshakePayload { identity_key = PublicKey protobuf, identity_sig
/// = Ed25519(STATIC_KEY_DOMAIN || static_public) }` — the exact bytes
/// libp2p-noise `send_identity` frames (no extensions on the golden path).
fn identity_payload(identity: &IdentityKeypair, static_public: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    field_bytes(1, &identity.public().encode_protobuf(), &mut payload);
    let signature = identity
        .sign(&[STATIC_KEY_DOMAIN, static_public].concat())
        .expect("ed25519 signing cannot fail");
    field_bytes(2, &signature, &mut payload);
    payload
}

/// u16-BE length prefix + payload — the wire frame shape shared by handshake
/// flights and transport frames (libp2p-noise `encode_length_prefixed`).
fn length_prefixed(bytes: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(2 + bytes.len());
    frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    frame.extend_from_slice(bytes);
    frame
}

/// The snow builder exactly as libp2p-noise `noise_params_into_builder`
/// composes it, plus the ephemeral pin.
fn xx_builder<'a>(
    static_secret: &'a [u8; 32],
    ephemeral_secret: &'a [u8; 32],
) -> Builder<'a> {
    let params: NoiseParams = PROTOCOL_NAME.parse().expect("valid Noise params");
    Builder::with_resolver(params, Box::new(RecorderResolver(RingResolver)))
        .prologue(&[])
        .local_private_key(static_secret)
        .fixed_ephemeral_key_for_testing_only(ephemeral_secret)
}

/// X25519 DH over `x25519-dalek` — mirrors libp2p-noise's own `Dh` impl
/// (`protocol.rs::Keypair`, which uses `x25519(secret, X25519_BASEPOINT_BYTES)`
/// for both `set` and `dh`), byte-for-byte.
struct X25519Dh {
    secret: [u8; 32],
    public: [u8; 32],
}

impl Default for X25519Dh {
    fn default() -> Self {
        Self {
            secret: [0u8; 32],
            public: [0u8; 32],
        }
    }
}

impl Dh for X25519Dh {
    fn name(&self) -> &'static str {
        "25519"
    }
    fn pub_len(&self) -> usize {
        32
    }
    fn priv_len(&self) -> usize {
        32
    }
    fn set(&mut self, privkey: &[u8]) {
        self.secret.copy_from_slice(&privkey[..32]);
        self.public = x25519(self.secret, X25519_BASEPOINT_BYTES);
    }
    fn generate(&mut self, _rng: &mut dyn Random) {
        // The recorder only ever runs with pinned keys
        // (`fixed_ephemeral_key_for_testing_only` skips `generate`); a
        // random-key run would silently diverge from the fixture contract,
        // so refuse instead.
        panic!("X25519Dh::generate: recorder requires pinned keys");
    }
    fn pubkey(&self) -> &[u8] {
        &self.public
    }
    fn privkey(&self) -> &[u8] {
        &self.secret
    }
    fn dh(&self, pubkey: &[u8], out: &mut [u8]) -> Result<(), snow::Error> {
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&pubkey[..32]);
        out[..32].copy_from_slice(&x25519(self.secret, pk));
        Ok(())
    }
}

/// Resolver mirroring libp2p-noise's `protocol.rs::Resolver`: X25519 DH from
/// x25519-dalek, everything else delegated to snow's `RingResolver` (which is
/// exactly what libp2p-noise itself delegates hash/cipher to).
struct RecorderResolver(RingResolver);

impl CryptoResolver for RecorderResolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        self.0.resolve_rng()
    }
    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        if matches!(choice, DHChoice::Curve25519) {
            Some(Box::new(X25519Dh::default()))
        } else {
            None
        }
    }
    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        self.0.resolve_hash(choice)
    }
    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        self.0.resolve_cipher(choice)
    }
}

fn main() {
    let init_identity =
        IdentityKeypair::ed25519_from_bytes(INIT_IDENTITY_SEED).expect("valid ed25519 seed");
    let resp_identity =
        IdentityKeypair::ed25519_from_bytes(RESP_IDENTITY_SEED).expect("valid ed25519 seed");

    // Static X25519 public keys — the bytes carried in the `s` token
    // plaintext and signed by the long-term identity (§4.2).
    let init_static_pub = x25519(INIT_STATIC, X25519_BASEPOINT_BYTES);
    let resp_static_pub = x25519(RESP_STATIC, X25519_BASEPOINT_BYTES);

    let mut initiator = xx_builder(&INIT_STATIC, &INIT_EPHEMERAL)
        .build_initiator()
        .expect("initiator session");
    let mut responder = xx_builder(&RESP_STATIC, &RESP_EPHEMERAL)
        .build_responder()
        .expect("responder session");

    let mut buf = [0u8; 65536];
    let mut plain = [0u8; 65536];

    // Flight 1: I → R, empty payload (rides in the clear — k is empty).
    let n = initiator.write_message(&[], &mut buf).expect("flight 1");
    let flight1 = buf[..n].to_vec();
    let m = responder
        .read_message(&flight1, &mut plain)
        .expect("read flight 1");
    assert_eq!(m, 0, "flight-1 payload must be empty");

    // Flight 2: R → I, responder identity payload.
    let payload2 = identity_payload(&resp_identity, &resp_static_pub);
    let n = responder
        .write_message(&payload2, &mut buf)
        .expect("flight 2");
    let flight2 = buf[..n].to_vec();
    let m = initiator
        .read_message(&flight2, &mut plain)
        .expect("read flight 2");
    assert_eq!(&plain[..m], &payload2[..], "initiator sees the responder payload");

    // Flight 3: I → R, initiator identity payload.
    let payload3 = identity_payload(&init_identity, &init_static_pub);
    let n = initiator
        .write_message(&payload3, &mut buf)
        .expect("flight 3");
    let flight3 = buf[..n].to_vec();
    let m = responder
        .read_message(&flight3, &mut plain)
        .expect("read flight 3");
    assert_eq!(&plain[..m], &payload3[..], "responder sees the initiator payload");

    // Both parties agree on the handshake hash; static keys crossed over.
    let handshake_hash = initiator.get_handshake_hash().to_vec();
    assert_eq!(handshake_hash, responder.get_handshake_hash());
    assert_eq!(initiator.get_remote_static(), Some(&resp_static_pub[..]));
    assert_eq!(responder.get_remote_static(), Some(&init_static_pub[..]));

    // Verify both identity signatures exactly like libp2p-noise `finish`
    // (`id_pk.verify(STATIC_KEY_DOMAIN || static_pub, sig)`).
    assert!(resp_identity.public().verify(
        &[STATIC_KEY_DOMAIN, &resp_static_pub].concat(),
        &payload2[payload2.len() - 64..],
    ));
    assert!(init_identity.public().verify(
        &[STATIC_KEY_DOMAIN, &init_static_pub].concat(),
        &payload3[payload3.len() - 64..],
    ));

    // Post-handshake transport: seal one frame in each direction and prove
    // the other side opens it (Split keys c1 / c2, nonce 0).
    let mut t_init = initiator.into_transport_mode().expect("initiator transport");
    let mut t_resp = responder.into_transport_mode().expect("responder transport");
    let n = t_init
        .write_message(TRANSPORT_PLAINTEXT, &mut buf)
        .expect("seal initiator→responder");
    let i2r_sealed = buf[..n].to_vec();
    let m = t_resp
        .read_message(&i2r_sealed, &mut plain)
        .expect("open initiator→responder");
    assert_eq!(&plain[..m], TRANSPORT_PLAINTEXT);
    let n = t_resp
        .write_message(TRANSPORT_PLAINTEXT, &mut buf)
        .expect("seal responder→initiator");
    let r2i_sealed = buf[..n].to_vec();
    let m = t_init
        .read_message(&r2i_sealed, &mut plain)
        .expect("open responder→initiator");
    assert_eq!(&plain[..m], TRANSPORT_PLAINTEXT);

    let fixture = serde_json::json!({
        "protocol": PROTOCOL_NAME,
        "prologue": "",
        "framing": "u16-BE length prefix; pure Noise frames after multistream (the /noise negotiation lives outside the Noise messages — contract §6)",
        "source": "rust-libp2p Noise stack recording: libp2p-noise 0.46.1 (libp2p 0.56.0) engine = snow 0.9.6, driven with the same builder parameters libp2p_noise::Config composes (noise_params_into_builder) and pinned static + ephemeral + identity keys; dev-only recorder crates/spoke-connect/examples/noise_recorder.rs",
        "roles": {
            "initiator": "I (dialer) — recorded flights 1 and 3; seals the initiator→responder transport frame",
            "responder": "R (listener) — the TS interop test plays this role"
        },
        "keys": {
            "initiator": {
                "identitySeed": hex(&INIT_IDENTITY_SEED),
                "identityPublic": hex(&ed25519_dalek::SigningKey::from_bytes(&INIT_IDENTITY_SEED).verifying_key().to_bytes()),
                "staticPrivate": hex(&INIT_STATIC),
                "staticPublic": hex(&init_static_pub),
                "ephemeralPrivate": hex(&INIT_EPHEMERAL),
                "ephemeralPublic": hex(&flight1[..32]),
                "peerId": init_identity.public().to_peer_id().to_base58(),
            },
            "responder": {
                "identitySeed": hex(&RESP_IDENTITY_SEED),
                "identityPublic": hex(&ed25519_dalek::SigningKey::from_bytes(&RESP_IDENTITY_SEED).verifying_key().to_bytes()),
                "staticPrivate": hex(&RESP_STATIC),
                "staticPublic": hex(&resp_static_pub),
                "ephemeralPrivate": hex(&RESP_EPHEMERAL),
                "ephemeralPublic": hex(&flight2[..32]),
                "peerId": resp_identity.public().to_peer_id().to_base58(),
            },
        },
        "flights": {
            "flight1": hex(&length_prefixed(&flight1)),
            "flight2": hex(&length_prefixed(&flight2)),
            "flight3": hex(&length_prefixed(&flight3)),
        },
        "payloads": {
            "flight1": "",
            "flight2": hex(&payload2),
            "flight3": hex(&payload3),
        },
        "handshake": {
            "handshakeHash": hex(&handshake_hash),
        },
        "transport": {
            "initiatorToResponder": {
                "frame": hex(&length_prefixed(&i2r_sealed)),
                "plaintext": hex(TRANSPORT_PLAINTEXT),
                "note": "spoke-over-noise (ASCII) — sealed under the initiator TX key (Split c1), nonce 0; opens with the responder RX key",
            },
            "responderToInitiator": {
                "frame": hex(&length_prefixed(&r2i_sealed)),
                "plaintext": hex(TRANSPORT_PLAINTEXT),
                "note": "spoke-over-noise (ASCII) — sealed under the responder TX key (Split c2), nonce 0; opens with the initiator RX key",
            },
        },
    });

    println!("{}", serde_json::to_string_pretty(&fixture).expect("json serialization"));
}
