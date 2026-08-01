/**
 * Ed25519 → peer_id derivation (SPOKE connect identity binding).
 *
 * Ported from tooling/connect-identity-proof/proof.mjs; normative formula
 * `.mstar/specs/spoke-connect.md` §Identity binding (mirrors
 * `crates/spoke-connect/src/core/peer_id.rs`). Do not invent a second
 * derivation — byte parity with the Rust/libp2p path is locked by the golden
 * vector tests.
 *
 * Derivation (protocol version 1):
 * 1. Encode the 32-byte Ed25519 public key as the libp2p protobuf
 *    `PublicKey` message: field 1 (`Type`) = 1 (Ed25519); field 2 (`Data`)
 *    = the raw key.
 * 2. Build the identity multihash (code `0x00`) over the protobuf bytes.
 *    Ed25519 protobuf keys are ≤ 42 bytes, so the digest is the protobuf
 *    bytes themselves — not a sha2-256 hash of them.
 * 3. Encode the multihash byte sequence with base58btc (Bitcoin alphabet).
 *    No multibase prefix, no CIDv1 wrapping.
 */

/** Bitcoin alphabet (base58btc), as used by libp2p's `bs58` encoding. */
const BITCOIN_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/**
 * base58btc encode (Bitcoin alphabet). Leading zero bytes map to leading
 * `1` characters (base58 convention). Pure integer arithmetic, ported from
 * `peer_id.rs::base58_encode` / `proof.mjs::base58Encode`.
 */
function base58Encode(input: Uint8Array): string {
  let zeros = 0;
  while (zeros < input.length && input[zeros] === 0) zeros++;

  // Little-endian base-58 digits (least significant first) — the classic
  // base-256 → base-58 conversion.
  const digits: number[] = [];
  for (let i = 0; i < input.length; i++) {
    let carry = input[i];
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j] << 8;
      digits[j] = carry % 58;
      carry = Math.floor(carry / 58);
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = Math.floor(carry / 58);
    }
  }

  let out = "1".repeat(zeros);
  for (let i = digits.length - 1; i >= 0; i--) {
    out += BITCOIN_ALPHABET[digits[i]];
  }
  return out;
}

/**
 * Derive the wire `peer_id` string for a 32-byte Ed25519 public key
 * (protobuf `PublicKey` → identity multihash `0x00` → base58btc).
 */
export function derivePeerIdFromEd25519Pubkey(pubkey: Uint8Array): string {
  if (!(pubkey instanceof Uint8Array) || pubkey.length !== 32) {
    throw new Error("pubkey must be 32 bytes");
  }

  // Protobuf PublicKey message, hand-encoded (fixed layout):
  //   field 1, varint Type = 1 (Ed25519):  tag 0x08, value 0x01
  //   field 2, bytes Data (32-byte key):   tag 0x12, length 0x20, key
  // 36 bytes total — always ≤ 42, so the identity multihash branch applies.
  const pkBytes = new Uint8Array(36);
  pkBytes[0] = 0x08;
  pkBytes[1] = 0x01;
  pkBytes[2] = 0x12;
  pkBytes[3] = 0x20;
  pkBytes.set(pubkey, 4);

  // Identity multihash: code-varint 0x00, length-varint 36 (0x24) — both
  // single-byte varints for this digest size — then the protobuf bytes.
  const multihash = new Uint8Array(38);
  multihash[0] = 0x00;
  multihash[1] = 0x24;
  multihash.set(pkBytes, 2);

  return base58Encode(multihash);
}
