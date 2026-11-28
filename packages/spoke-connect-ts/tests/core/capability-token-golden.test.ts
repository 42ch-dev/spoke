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
  audiencePeerIdFromRoleSeed,
  subjectPeerIdFromRoleSeed,
} from "../golden-role-peer-ids.js";
import {
  GOLDEN_PEER_ID,
  GOLDEN_SEED,
} from "../fixtures/golden.js";
import {
  TOKEN_VERSION,
  verifyCapabilityToken,
} from "../../src/core/capability-token.js";
import type { CapabilityTokenProof } from "../../src/core/capability-token.js";

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
    expect(goldenProof.claims.iss).toBe(GOLDEN_PEER_ID);
    expect(
      derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(GOLDEN_SEED)),
    ).toBe(GOLDEN_PEER_ID);
    const subject = subjectPeerIdFromRoleSeed();
    const audience = audiencePeerIdFromRoleSeed();
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
    expect(goldenProof.sig).toHaveLength(86);
    expect(base64UrlEncode(base64UrlDecode(goldenProof.sig))).toBe(goldenProof.sig);
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
