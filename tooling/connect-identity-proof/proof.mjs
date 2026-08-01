#!/usr/bin/env node
/**
 * Throwaway identity-byte reproducibility proof (Node, zero deps).
 *
 * Golden vectors mirror crates/spoke-connect/src/core/{peer_id,hello_crypto}.rs
 * (seed 1..=32 → pubkey 79b5562e… → peer_id 12D3KooWJ1T…; JCS hex + signature).
 *
 * Normative: .mstar/specs/spoke-connect.md §Identity binding / §Signature canonicalization
 *
 * Usage: node tooling/connect-identity-proof/proof.mjs
 */

import { webcrypto } from "node:crypto";
import { strict as assert } from "node:assert";

const subtle = webcrypto.subtle;

// ── Golden vectors (from Rust core tests) ──────────────────────────────────

/** Ed25519 seed bytes 1..=32 */
const GOLDEN_SEED = Uint8Array.from({ length: 32 }, (_, i) => i + 1);

const GOLDEN_PUBKEY = Uint8Array.from([
  0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9, 0x40, 0x78, 0xb1, 0x12, 0xe8,
  0xa9, 0x8b, 0xa7, 0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7, 0xe0, 0xe3,
  0x91, 0x0b, 0xad, 0x04, 0x96, 0x64,
]);

const GOLDEN_PEER_ID =
  "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf";

const GOLDEN_NONCE = "golden-nonce-000000000001";

/**
 * RFC 8785 JCS of signed object {protocol_version, peer_id, nonce, host}.
 * host.authority is omitted (not null) — matches Rust skip_serializing_if.
 */
const GOLDEN_JCS_HEX =
  "7b22686f7374223a7b226361706162696c6974696573223a5b2273706f6b652d626173656c696e65225d2c22657874656e73696f6e73223a7b7d2c22686f73745f6964223a22676f6c64656e2d686f7374222c226e616d65737061636573223a5b5d2c22726f6c6573223a5b22646174612d73746f7265225d2c22736368656d615f76657273696f6e223a317d2c226e6f6e6365223a22676f6c64656e2d6e6f6e63652d303030303030303030303031222c22706565725f6964223a22313244334b6f6f574a315473696a48374835463734686641443558697368517a337378726d41745659333747744e643943715966222c2270726f746f636f6c5f76657273696f6e223a317d";

/** base64url (no pad) of raw 64-byte Ed25519 signature over GOLDEN_JCS_HEX */
const GOLDEN_SIGNATURE =
  "yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg";

const PROTOCOL_VERSION = 1;

/** Golden host manifest — authority absent (omit, do not emit null). */
function goldenManifest() {
  return {
    capabilities: ["spoke-baseline"],
    extensions: {},
    host_id: "golden-host",
    namespaces: [],
    roles: ["data-store"],
    schema_version: 1,
  };
}

// ── base58btc (Bitcoin alphabet) ───────────────────────────────────────────

const BITCOIN_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

function base58Encode(input) {
  let zeros = 0;
  while (zeros < input.length && input[zeros] === 0) zeros++;

  const digits = [];
  for (let i = 0; i < input.length; i++) {
    let carry = input[i];
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j] << 8;
      digits[j] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }

  let out = "1".repeat(zeros);
  for (let i = digits.length - 1; i >= 0; i--) {
    out += BITCOIN_ALPHABET[digits[i]];
  }
  return out;
}

// ── peer_id derivation (spec §Identity binding) ────────────────────────────

/**
 * Wire peer_id from 32-byte Ed25519 public key:
 * protobuf PublicKey{Type=Ed25519, Data=pubkey} → identity multihash 0x00 → base58btc
 */
function derivePeerIdFromEd25519Pubkey(pubkey) {
  if (!(pubkey instanceof Uint8Array) || pubkey.length !== 32) {
    throw new Error("pubkey must be 32 bytes");
  }
  // field 1 varint Type=1: 0x08 0x01; field 2 bytes Data len 32: 0x12 0x20 + key
  const pkBytes = new Uint8Array(36);
  pkBytes[0] = 0x08;
  pkBytes[1] = 0x01;
  pkBytes[2] = 0x12;
  pkBytes[3] = 0x20;
  pkBytes.set(pubkey, 4);

  // identity multihash: code 0x00, length 36 (0x24), digest = pkBytes
  const multihash = new Uint8Array(38);
  multihash[0] = 0x00;
  multihash[1] = 0x24;
  multihash.set(pkBytes, 2);

  return base58Encode(multihash);
}

// ── RFC 8785 JCS (subset sufficient for connect hello signed objects) ──────

/**
 * Canonicalize a JSON value per RFC 8785 for plain objects used by connect:
 * objects, arrays, strings, integers, booleans, null. No IEEE floats.
 * `undefined` object members are omitted (not serialized as null).
 */
function jcs(value) {
  if (value === null) return "null";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") {
    if (!Number.isInteger(value)) {
      throw new Error(`JCS proof rejects non-integer number: ${value}`);
    }
    if (Object.is(value, -0)) return "0";
    return String(value);
  }
  if (typeof value === "string") {
    // JSON.stringify produces RFC 8259 string encoding; JCS uses the same
    // escaping rules for strings without requiring \u escapes for BMP.
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return "[" + value.map(jcs).join(",") + "]";
  }
  if (typeof value === "object") {
    const keys = Object.keys(value)
      .filter((k) => value[k] !== undefined)
      .sort();
    return (
      "{" +
      keys.map((k) => JSON.stringify(k) + ":" + jcs(value[k])).join(",") +
      "}"
    );
  }
  throw new Error(`unsupported JCS type: ${typeof value}`);
}

function utf8Bytes(str) {
  return new TextEncoder().encode(str);
}

function toHex(bytes) {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function fromHex(hex) {
  if (hex.length % 2 !== 0) throw new Error("odd hex length");
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

// ── base64url (no padding) ─────────────────────────────────────────────────

function base64UrlEncode(bytes) {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  const b64 = btoa(bin);
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function base64UrlDecode(s) {
  const pad = s.length % 4 === 0 ? "" : "=".repeat(4 - (s.length % 4));
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/") + pad;
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

// ── WebCrypto Ed25519 (PKCS8 seed import / SPKI pubkey import) ─────────────

/** PKCS8 wrapping a raw 32-byte Ed25519 seed (RFC 8410). */
function ed25519SeedToPkcs8(seed) {
  // 30 2e 02 01 00 30 05 06 03 2b 65 70 04 22 04 20 || seed
  const header = Uint8Array.from([
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70,
    0x04, 0x22, 0x04, 0x20,
  ]);
  const out = new Uint8Array(header.length + seed.length);
  out.set(header, 0);
  out.set(seed, header.length);
  return out;
}

/** SPKI wrapping a raw 32-byte Ed25519 public key (RFC 8410). */
function ed25519PubkeyToSpki(pubkey) {
  // 30 2a 30 05 06 03 2b 65 70 03 21 00 || pubkey
  const header = Uint8Array.from([
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
  ]);
  const out = new Uint8Array(header.length + pubkey.length);
  out.set(header, 0);
  out.set(pubkey, header.length);
  return out;
}

async function importSigningKey(seed) {
  return subtle.importKey(
    "pkcs8",
    ed25519SeedToPkcs8(seed),
    { name: "Ed25519" },
    false,
    ["sign"],
  );
}

async function importVerifyKey(pubkey) {
  return subtle.importKey(
    "spki",
    ed25519PubkeyToSpki(pubkey),
    { name: "Ed25519" },
    false,
    ["verify"],
  );
}

async function signEd25519(seed, message) {
  const key = await importSigningKey(seed);
  const sig = await subtle.sign({ name: "Ed25519" }, key, message);
  return new Uint8Array(sig);
}

async function verifyEd25519(pubkey, message, signature) {
  const key = await importVerifyKey(pubkey);
  return subtle.verify({ name: "Ed25519" }, key, signature, message);
}

// ── Checks ─────────────────────────────────────────────────────────────────

function check(name, ok, detail) {
  const mark = ok ? "PASS" : "FAIL";
  console.log(`[${mark}] ${name}${detail ? ` — ${detail}` : ""}`);
  return ok;
}

async function main() {
  console.log("connect-identity-proof — JS reproducibility of SPOKE connect identity bytes");
  console.log("Node", process.version);
  console.log("");

  let allOk = true;

  // 1. peer_id
  const peerId = derivePeerIdFromEd25519Pubkey(GOLDEN_PUBKEY);
  const peerOk = peerId === GOLDEN_PEER_ID;
  allOk =
    check(
      "peer_id derivation",
      peerOk,
      peerOk ? peerId : `got ${peerId}, want ${GOLDEN_PEER_ID}`,
    ) && allOk;

  // 2. JCS
  const signedObject = {
    protocol_version: PROTOCOL_VERSION,
    peer_id: GOLDEN_PEER_ID,
    nonce: GOLDEN_NONCE,
    host: goldenManifest(),
  };
  // JCS sorts keys; build via object then canonicalize
  const jcsStr = jcs(signedObject);
  const jcsBytes = utf8Bytes(jcsStr);
  const jcsHex = toHex(jcsBytes);
  const jcsOk = jcsHex === GOLDEN_JCS_HEX;
  allOk =
    check(
      "JCS UTF-8 bytes",
      jcsOk,
      jcsOk
        ? `${jcsBytes.length} bytes match golden hex`
        : `got ${jcsHex.slice(0, 40)}… want ${GOLDEN_JCS_HEX.slice(0, 40)}…`,
    ) && allOk;
  if (jcsOk) {
    console.log(`       jcs utf8: ${jcsStr}`);
  }

  // 3 + 4. Sign + base64url
  let signOk = false;
  let b64Ok = false;
  let sigBytes;
  try {
    sigBytes = await signEd25519(GOLDEN_SEED, jcsBytes);
    signOk = sigBytes.length === 64;
    const b64 = base64UrlEncode(sigBytes);
    b64Ok = b64 === GOLDEN_SIGNATURE;
    allOk =
      check(
        "Ed25519 sign (64 raw bytes)",
        signOk,
        signOk ? "64 bytes" : `got length ${sigBytes.length}`,
      ) && allOk;
    allOk =
      check(
        "base64url (no pad) signature",
        b64Ok,
        b64Ok ? b64 : `got ${b64}, want ${GOLDEN_SIGNATURE}`,
      ) && allOk;
  } catch (e) {
    allOk = check("Ed25519 sign (WebCrypto)", false, String(e)) && allOk;
    allOk = check("base64url (no pad) signature", false, "skipped (sign failed)") && allOk;
  }

  // 5. Verify golden signature over golden JCS
  const goldenJcsBytes = fromHex(GOLDEN_JCS_HEX);
  const goldenSigBytes = base64UrlDecode(GOLDEN_SIGNATURE);
  let verifyOk = false;
  try {
    verifyOk = await verifyEd25519(
      GOLDEN_PUBKEY,
      goldenJcsBytes,
      goldenSigBytes,
    );
    allOk =
      check(
        "Ed25519 verify golden signature",
        verifyOk,
        verifyOk ? "verified" : "WebCrypto reject",
      ) && allOk;
  } catch (e) {
    allOk =
      check("Ed25519 verify golden signature", false, String(e)) && allOk;
  }

  // Sanity: our JCS bytes equal golden hex decode
  const jcsBytesMatchGolden = toHex(jcsBytes) === toHex(goldenJcsBytes);
  allOk =
    check(
      "local JCS equals golden hex decode",
      jcsBytesMatchGolden,
    ) && allOk;

  console.log("");
  if (allOk) {
    console.log("RESULT: ALL CHECKS PASSED");
    console.log(
      "Identity binding (peer_id + JCS + Ed25519 + base64url) is reproducible in JS via pure-TS algorithms + WebCrypto.",
    );
    process.exit(0);
  } else {
    console.log("RESULT: FAILURE — do not lock a TS connect route on broken identity mapping.");
    console.log("Escalate to P0 (normative identity / JCS field rules) before T3 recommendation.");
    process.exit(1);
  }
}

main().catch((e) => {
  console.error("fatal:", e);
  process.exit(2);
});
