import { describe, expect, it } from "vitest";

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import { GOLDEN_PEER_ID, GOLDEN_SEED } from "../../src/golden.js";
import {
  issueCapabilityToken,
  verifyCapabilityToken,
} from "../../src/core/capability-token.js";
import type {
  CapabilityClaims,
  CapabilityTokenProof,
} from "../../src/core/capability-token.js";

/**
 * TS-minted capability-token golden vector: deterministic proof from the
 * identity golden seed (`tooling/connect-identity-proof/`, bytes 1..=32),
 * checked in as `tests/fixtures/capability-token-ts-golden.json` (byte-identical
 * registered copy of the SSOT under `crates/spoke-connect/tests/fixtures/`; sync
 * gate: `tooling/connect/golden-vector-sync.mjs`).
 *
 * The vitest drift guard asserts the minted proof matches the committed
 * JSON (including provenance metadata). Consumers strip `provenance` before
 * strict parse/verify — see plan strict-parser note.
 */

const NOW = 1_000_000_000;

const PROVENANCE = {
  minted_by: "typescript",
  minted_with: "@42ch/spoke-connect issueCapabilityToken",
  seed: "identity-golden-1..32",
} as const;

const fixtureUrl = new URL(
  "../../tests/fixtures/capability-token-ts-golden.json",
  import.meta.url,
);

interface TsMintedGoldenVector extends CapabilityTokenProof {
  provenance: typeof PROVENANCE;
}

function subjectPeerId(): string {
  return derivePeerIdFromEd25519Pubkey(
    getPublicKeyEd25519(new Uint8Array(32).fill(7)),
  );
}

function audiencePeerId(): string {
  return derivePeerIdFromEd25519Pubkey(
    getPublicKeyEd25519(new Uint8Array(32).fill(8)),
  );
}

function goldenClaims(): CapabilityClaims {
  return {
    iss: GOLDEN_PEER_ID,
    sub: subjectPeerId(),
    aud: audiencePeerId(),
    capabilities: ["spoke-baseline", "l2-computable"],
    exp: NOW + 3600,
    iat: NOW,
    jti: "golden-jti-001",
  };
}

async function mintTsGoldenVector(): Promise<TsMintedGoldenVector> {
  const proof = await issueCapabilityToken(
    GOLDEN_SEED,
    goldenClaims(),
    NOW,
  );
  return { ...proof, provenance: { ...PROVENANCE } };
}

function loadCommittedVector(): TsMintedGoldenVector {
  return JSON.parse(
    readFileSync(fileURLToPath(fixtureUrl), "utf8"),
  ) as TsMintedGoldenVector;
}

function proofWithoutProvenance(
  vector: TsMintedGoldenVector,
): CapabilityTokenProof {
  const { v, claims, sig } = vector;
  return { v, claims, sig };
}

describe("TS-minted capability-token golden vector", () => {
  it("matches the committed SSOT fixture (drift guard)", async () => {
    const minted = await mintTsGoldenVector();
    const committed = loadCommittedVector();
    expect(minted).toEqual(committed);
  });

  it("self-verifies after stripping provenance", async () => {
    const committed = loadCommittedVector();
    const proof = proofWithoutProvenance(committed);
    const granted = await verifyCapabilityToken(
      proof,
      [GOLDEN_PEER_ID],
      audiencePeerId(),
      subjectPeerId(),
      NOW,
    );
    expect(granted).toEqual(["spoke-baseline", "l2-computable"]);
  });
});
