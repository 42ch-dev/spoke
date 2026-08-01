//! Pure Ed25519 → `peer_id` string derivation (identity binding, protocol
//! version 1).
//!
//! Normative mapping (`.mstar/specs/spoke-connect.md` §Identity binding):
//!
//! 1. Encode the public key as the libp2p protobuf **`PublicKey`** message:
//!    field 1 (`Type`) = 1 (**Ed25519**); field 2 (`Data`) = the 32-byte raw
//!    key.
//! 2. Build the **identity** multihash (code `0x00`) over the protobuf bytes:
//!    Ed25519 protobuf keys are ≤ 42 bytes, so the digest is the protobuf
//!    bytes themselves — **not** a sha2-256 hash of them.
//! 3. Encode the multihash byte sequence with **base58btc** (Bitcoin
//!    alphabet). No multibase prefix, no CIDv1 wrapping.
//!
//! The result matches rust-libp2p `PeerId::to_string()` for the same key
//! (locked by the golden vector test).

/// Bitcoin alphabet (base58btc), as used by libp2p's `bs58` encoding.
const BITCOIN_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Derive the wire `peer_id` string for an Ed25519 public key.
#[must_use]
pub fn derive_peer_id_from_ed25519_pubkey(pubkey: &[u8; 32]) -> String {
    // Spec §Identity binding, step 1 (protobuf `PublicKey` message),
    // hand-encoded (fixed layout):
    //   field 1, varint Type = 1 (Ed25519):    tag 0x08, value 0x01
    //   field 2, bytes Data (32-byte key):     tag 0x12, length 0x20, key
    // 36 bytes total — always ≤ 42, so the identity multihash branch applies.
    let mut pk_bytes = [0u8; 36];
    pk_bytes[0] = 0x08;
    pk_bytes[1] = 0x01;
    pk_bytes[2] = 0x12;
    pk_bytes[3] = 0x20;
    pk_bytes[4..].copy_from_slice(pubkey);

    // Spec §Identity binding, step 3 (identity multihash): code-varint 0x00,
    // length-varint 36 (0x24) — both single-byte varints for this digest
    // size — then the protobuf bytes as the digest.
    let mut multihash = [0u8; 38];
    multihash[0] = 0x00;
    multihash[1] = 0x24;
    multihash[2..].copy_from_slice(&pk_bytes);

    base58_encode(&multihash)
}

/// Invert [`derive_peer_id_from_ed25519_pubkey`]: decode a well-formed
/// Ed25519 `peer_id` string back to its 32-byte public key.
///
/// Ed25519 peer ids use the **identity** multihash (digest = the key's
/// protobuf bytes), so this is a pure encoding inversion — no hash to
/// invert. `None` for any string that is not a structurally valid Ed25519
/// peer id (bad base58, wrong multihash layout, non-Ed25519 protobuf
/// header). Used by capability-token verification: the issuer `peer_id`
/// (`claims.iss`) carries the issuer's public key, which verifies the
/// token signature.
#[must_use]
pub fn ed25519_pubkey_from_peer_id(peer_id: &str) -> Option<[u8; 32]> {
    let bytes = base58_decode(peer_id)?;
    // Identity multihash (code 0x00, length 0x24 = 36) with the protobuf
    // `PublicKey` message as the digest (38 bytes total).
    if bytes.len() != 38 || bytes[0] != 0x00 || bytes[1] != 0x24 {
        return None;
    }
    // Protobuf `PublicKey`: field 1 varint Type = 1 (Ed25519): 0x08 0x01;
    // field 2 bytes, length 0x20 (32): 0x12 0x20, then the raw key.
    let message = &bytes[2..];
    if message.len() != 36
        || message[0] != 0x08
        || message[1] != 0x01
        || message[2] != 0x12
        || message[3] != 0x20
    {
        return None;
    }
    let mut pubkey = [0u8; 32];
    pubkey.copy_from_slice(&message[4..]);
    Some(pubkey)
}

/// Spec §Identity binding, step 4: base58btc (Bitcoin alphabet) encode
/// `input` with pure `std` arithmetic.
///
/// Leading zero bytes map to leading `1` characters (base58 convention).
/// Matches `bs58`'s default alphabet, which is what libp2p uses for
/// `PeerId::to_base58()`.
fn base58_encode(input: &[u8]) -> String {
    let zeros = input.iter().take_while(|&&b| b == 0).count();
    // Little-endian base-58 digits (least significant first) — the classic
    // base-256 → base-58 conversion.
    let mut digits: Vec<u8> = Vec::with_capacity(input.len() * 2);
    for &byte in input {
        let mut carry = u32::from(byte);
        for digit in digits.iter_mut() {
            carry += u32::from(*digit) << 8;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(input.len() * 2);
    for _ in 0..zeros {
        out.push('1');
    }
    for digit in digits.iter().rev() {
        out.push(BITCOIN_ALPHABET[usize::from(*digit)] as char);
    }
    out
}

/// Decode a base58btc string with pure `std` arithmetic (inverse of
/// [`base58_encode`]). `None` on any character outside the Bitcoin
/// alphabet.
fn base58_decode(input: &str) -> Option<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::with_capacity(input.len());
    for character in input.bytes() {
        let digit = BITCOIN_ALPHABET.iter().position(|&a| a == character)?;
        // Accumulate little-endian base-256 digits: multiply the running
        // number by 58 and add the new digit (inverse of the encoder).
        let mut carry = digit as u32;
        for byte in bytes.iter_mut() {
            carry += u32::from(*byte) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    bytes.reverse();
    // Leading `1` characters encode leading zero bytes (base58 convention).
    let zeros = input.bytes().take_while(|&b| b == b'1').count();
    for _ in 0..zeros {
        bytes.insert(0, 0);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden vector captured from rust-libp2p (`PeerId::to_string()` over
    /// `Keypair::ed25519_from_bytes(seed)`) for the Ed25519 key seeded with
    /// bytes 1..=32 — captured against libp2p-identity 0.2.14 / libp2p 0.56
    /// before the transport cutover.
    const GOLDEN_PUBKEY: [u8; 32] = [
        0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9, 0x40, 0x78, 0xb1, 0x12, 0xe8, 0xa9, 0x8b,
        0xa7, 0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7, 0xe0, 0xe3, 0x91, 0x0b, 0xad, 0x04,
        0x96, 0x64,
    ];
    const GOLDEN_PEER_ID: &str = "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf";

    #[test]
    fn golden_peer_id_matches_libp2p_derivation() {
        assert_eq!(
            derive_peer_id_from_ed25519_pubkey(&GOLDEN_PUBKEY),
            GOLDEN_PEER_ID
        );
    }

    #[test]
    fn distinct_keys_derive_distinct_peer_ids() {
        let a = derive_peer_id_from_ed25519_pubkey(&[1u8; 32]);
        let b = derive_peer_id_from_ed25519_pubkey(&[2u8; 32]);
        assert_ne!(a, b);
    }

    #[test]
    fn ed25519_peer_ids_share_the_identity_multihash_prefix() {
        // The first bytes of the multihash (identity code + fixed protobuf
        // header) are constant for every Ed25519 key, so every derived peer
        // id shares the `12D3KooW` prefix — the well-known libp2p Ed25519
        // peer id shape.
        let a = derive_peer_id_from_ed25519_pubkey(&[3u8; 32]);
        assert!(a.starts_with("12D3KooW"));
        assert_eq!(a.len(), GOLDEN_PEER_ID.len());
    }

    #[test]
    fn golden_peer_id_decodes_back_to_its_public_key() {
        // The identity multihash makes the mapping an inversion: the golden
        // peer id (captured from libp2p) decodes to exactly the golden
        // public key, and that key derives the same peer id.
        assert_eq!(
            ed25519_pubkey_from_peer_id(GOLDEN_PEER_ID),
            Some(GOLDEN_PUBKEY)
        );
    }

    #[test]
    fn pubkey_peer_id_round_trip_for_arbitrary_keys() {
        for seed in [4u8, 5, 6] {
            let pubkey = [seed; 32];
            let peer_id = derive_peer_id_from_ed25519_pubkey(&pubkey);
            assert_eq!(
                ed25519_pubkey_from_peer_id(&peer_id),
                Some(pubkey),
                "derive → decode must invert for the same key"
            );
        }
    }

    #[test]
    fn base58_decode_inverts_encode() {
        // Round-trip the encoder's output through the decoder for a few byte
        // patterns, including leading zero bytes (base58 `1`s).
        for input in [
            &[0u8; 0][..],
            &[0x01u8][..],
            &[0x00u8, 0x01][..],
            &[0xde, 0xad, 0xbe, 0xef][..],
        ] {
            let encoded = base58_encode(input);
            assert_eq!(
                base58_decode(&encoded).as_deref(),
                Some(input),
                "encode → decode must invert for {input:?}"
            );
        }
        // The classic base58btc example: 0x00 0x01 encodes to "12".
        assert_eq!(base58_encode(&[0x00, 0x01]), "12");
        assert_eq!(base58_decode("12").as_deref(), Some(&[0x00, 0x01][..]));
    }

    #[test]
    fn base58_decode_rejects_out_of_alphabet_input() {
        // '0', 'O', 'I', 'l' are excluded from the Bitcoin alphabet.
        for bad in ["0", "O", "I", "l", "12O3"] {
            assert_eq!(base58_decode(bad), None, "{bad:?} must not decode");
        }
    }

    #[test]
    fn pubkey_decode_rejects_non_ed25519_peer_id_shapes() {
        // A structurally valid base58 string that is not the identity
        // multihash of an Ed25519 key (wrong length / layout) decodes to
        // nothing — capability-token verification must fail closed.
        assert_eq!(
            ed25519_pubkey_from_peer_id("QmYwAPJzv5CZsnAzt8auVZRnV"),
            None
        );
        assert_eq!(ed25519_pubkey_from_peer_id("12D3KooW"), None);
        assert_eq!(ed25519_pubkey_from_peer_id(""), None);
    }
}
