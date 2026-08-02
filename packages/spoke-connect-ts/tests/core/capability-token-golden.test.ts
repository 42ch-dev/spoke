import { describe, expect, it } from "vitest";

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import {
  base64UrlDecode,
  base64UrlEncode,
  getPublicKeyEd25519,
} from "../../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  GOLDEN_PEER_ID,
  GOLDEN_SEED,
} from "../../src/golden.js";
import {
  TOKEN_VERSION,
  verifyCapabilityToken,
} from "../../src/core/capability-token.js";
import type { CapabilityTokenProof } from "../../src/core/capability-token.js";

/**
 * Cross-language golden vector: a capability token minted by the **Rust**
 * reference (`crates/spoke-connect`) from the identity golden seed
 * (`tooling/connect-identity-proof/`, bytes 1..=32), checked in as
 * `tests/fixtures/capability-token-golden.json`.
 *
 * Verifying the Rust-produced proof end-to-end on TS pins the JCS bytes TS
 * canonicalizes from `claims` to exactly what Rust signed, and the base64url
 * sig to the encoding Rust emits. It catches coordinated cross-language
 * drift in canonicalization / signature encoding that same-language
 * round-trip tests cannot see.
 *
 * Regeneration (temporary, uncommitted — do not land Rust changes):
 *   1. Add a one-off test at `crates/spoke-connect/tests/mint_golden.rs`
 *      that issues a token with the exact claims in the fixture below
 *      (issuer seed = `GOLDEN_SEED`, `iat` and `jti` present) and prints
 *      `serde_json::to_string_pretty(&proof)`.
 *   2. Run: `cargo +nightly test -p spoke-connect --test mint_golden -- --nocapture`
 *   3. Copy the printed proof into `tests/fixtures/capability-token-golden.json`
 *      and delete the temporary test.
 */

const NOW = 1_000_000_000;

const fixtureUrl = new URL(
  "../../tests/fixtures/capability-token-golden.json",
  import.meta.url,
);
const goldenProof = JSON.parse(
  readFileSync(fileURLToPath(fixtureUrl), "utf8"),
) as CapabilityTokenProof;

describe("Rust-minted capability-token golden vector", () => {
  it("verifies the Rust-produced proof (JCS + signature parity)", async () => {
    expect(goldenProof.v).toBe(TOKEN_VERSION);
    // The fixture issuer derives from the identity golden seed; sub/aud from
    // the deterministic role seeds [7u8; 32] / [8u8; 32] (same roles as the
    // Rust and TS unit tests).
    expect(goldenProof.claims.iss).toBe(GOLDEN_PEER_ID);
    expect(
      derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(GOLDEN_SEED)),
    ).toBe(GOLDEN_PEER_ID);
    const subject = derivePeerIdFromEd25519Pubkey(
      getPublicKeyEd25519(new Uint8Array(32).fill(7)),
    );
    const audience = derivePeerIdFromEd25519Pubkey(
      getPublicKeyEd25519(new Uint8Array(32).fill(8)),
    );
    expect(goldenProof.claims.sub).toBe(subject);
    expect(goldenProof.claims.aud).toBe(audience);

    const granted = await verifyCapabilityToken(
      goldenProof,
      [GOLDEN_PEER_ID],
      audience,
      subject,
      NOW,
    );
    expect(granted).toEqual(["spoke-baseline", "l2-computable"]);
  });

  it("carries a canonical unpadded base64url signature over the normative claim set", async () => {
    // 64 raw signature bytes → 86 base64url chars, no padding.
    expect(goldenProof.sig).toHaveLength(86);
    expect(base64UrlEncode(base64UrlDecode(goldenProof.sig))).toBe(goldenProof.sig);
    // iat / jti present in the fixture — the optional fields must survive
    // the Rust → TS round trip (omit-vs-null parity).
    const claimKeys = Object.keys(goldenProof.claims).sort();
    expect(claimKeys).toEqual([
      "aud",
      "capabilities",
      "exp",
      "iat",
      "iss",
      "jti",
      "sub",
    ]);
    expect(goldenProof.claims.exp).toBe(NOW + 3600);
    expect(goldenProof.claims.iat).toBe(NOW);
  });
});
