import { describe, expect, it } from "vitest";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import {
  derivePeerIdFromEd25519Pubkey,
  ed25519PubkeyFromPeerId,
} from "../../src/identity.js";
import { GOLDEN_PEER_ID, GOLDEN_PUBKEY } from "../../src/golden.js";
import {
  CLOCK_SKEW_SECONDS,
  TOKEN_VERSION,
  issueCapabilityToken,
  verifyCapabilityToken,
} from "../../src/core/capability-token.js";
import type {
  CapabilityClaims,
  CapabilityTokenProof,
} from "../../src/core/capability-token.js";

/**
 * Capability-token parity tests — mirror
 * `crates/spoke-connect/src/core/capability_token.rs` unit tests
 * (deterministic seeds per role, same expiry boundary / skew semantics).
 *
 * No Rust-produced golden capability-token vector is checked in, so inputs
 * are the deterministic seeds from the Rust tests: issuer `[1u8; 32]`,
 * other-issuer `[2u8; 32]`, subject `[7u8; 32]`, audience `[8u8; 32]`, and
 * `now = 1_000_000_000`. Behavioral parity (accept/reject outcomes and the
 * validation order) is the acceptance bar, not byte-identity to a TS vector.
 */

/** Deterministic test keys: distinct 32-byte seeds per role (Rust constants). */
const ISSUER_SEED = new Uint8Array(32).fill(1);
const OTHER_ISSUER_SEED = new Uint8Array(32).fill(2);

const NOW = 1_000_000_000;

/** A peer id for a raw Ed25519 seed (runtime-derived — no literals in crypto positions). */
function peerOf(seed: Uint8Array): string {
  return derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed));
}

function claims(
  iss: string,
  sub: string,
  aud: string,
  capabilities: string[],
  exp: number,
): CapabilityClaims {
  return { iss, sub, aud, capabilities, exp };
}

function trusted(issuers: string[]): string[] {
  return issuers;
}

/** A valid (issuer, subject, audience) triple for the happy path. */
async function happyToken(
  now: number,
  capabilities: string[],
): Promise<[CapabilityTokenProof, string[], [string, string]]> {
  const issuer = peerOf(ISSUER_SEED);
  const subject = peerOf(new Uint8Array(32).fill(7));
  const audience = peerOf(new Uint8Array(32).fill(8));
  const proof = await issueCapabilityToken(
    ISSUER_SEED,
    claims(issuer, subject, audience, capabilities, now + 3600),
  );
  return [proof, trusted([issuer]), [subject, audience]];
}

const tokenInvalid = expect.objectContaining({ code: "token_invalid" });

describe("issueCapabilityToken / verifyCapabilityToken (port of capability_token.rs)", () => {
  it("round-trips issuance to verification and returns the grant", async () => {
    const [proof, trustedIssuers, peers] = await happyToken(NOW, [
      "spoke-baseline",
      "l2-computable",
    ]);
    const granted = await verifyCapabilityToken(
      proof,
      trustedIssuers,
      peers[1],
      peers[0],
      NOW,
    );
    expect(granted).toEqual(["spoke-baseline", "l2-computable"]);
  });

  it("rejects an expired token at the expiry boundary", async () => {
    const issuer = peerOf(ISSUER_SEED);
    const subject = peerOf(new Uint8Array(32).fill(7));
    const audience = peerOf(new Uint8Array(32).fill(8));
    // exp == now is already expired (reject if now >= exp).
    const proof = await issueCapabilityToken(
      ISSUER_SEED,
      claims(issuer, subject, audience, ["spoke-baseline"], NOW),
    );
    await expect(
      verifyCapabilityToken(proof, trusted([issuer]), audience, subject, NOW),
    ).rejects.toThrowError(tokenInvalid);
    // And any time after exp.
    await expect(
      verifyCapabilityToken(
        proof,
        trusted([issuer]),
        audience,
        subject,
        NOW + 1,
      ),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("rejects an untrusted issuer", async () => {
    const [proof, , peers] = await happyToken(NOW, ["spoke-baseline"]);
    const other = peerOf(OTHER_ISSUER_SEED);
    await expect(
      verifyCapabilityToken(proof, trusted([other]), peers[1], peers[0], NOW),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("rejects every proof with an empty trusted-issuers list (method disabled)", async () => {
    const [proof, , peers] = await happyToken(NOW, ["spoke-baseline"]);
    await expect(
      verifyCapabilityToken(proof, [], peers[1], peers[0], NOW),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("rejects a wrong subject", async () => {
    const [proof, trustedIssuers, peers] = await happyToken(NOW, [
      "spoke-baseline",
    ]);
    const otherPeer = peerOf(new Uint8Array(32).fill(9));
    await expect(
      verifyCapabilityToken(proof, trustedIssuers, peers[1], otherPeer, NOW),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("rejects a wrong audience", async () => {
    const [proof, trustedIssuers, peers] = await happyToken(NOW, [
      "spoke-baseline",
    ]);
    const otherPeer = peerOf(new Uint8Array(32).fill(10));
    await expect(
      verifyCapabilityToken(proof, trustedIssuers, otherPeer, peers[0], NOW),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("rejects a malformed signature (not base64url; not 64 bytes)", async () => {
    const [proof, trustedIssuers, peers] = await happyToken(NOW, [
      "spoke-baseline",
    ]);

    const badBase64: CapabilityTokenProof = {
      ...proof,
      sig: "%%%not-base64url%%%",
    };
    await expect(
      verifyCapabilityToken(badBase64, trustedIssuers, peers[1], peers[0], NOW),
    ).rejects.toThrowError(tokenInvalid);

    // Valid base64url but not 64 bytes.
    const shortSig: CapabilityTokenProof = {
      ...proof,
      sig: "AA",
    };
    await expect(
      verifyCapabilityToken(shortSig, trustedIssuers, peers[1], peers[0], NOW),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("rejects tampered claims via signature verification", async () => {
    const [proof, trustedIssuers, peers] = await happyToken(NOW, [
      "spoke-baseline",
    ]);
    // Re-signing is not possible without the issuer key; mutating a claim
    // after issuance must break the signature check.
    const tampered: CapabilityTokenProof = {
      ...proof,
      claims: { ...proof.claims, capabilities: ["l2-computable"] },
    };
    await expect(
      verifyCapabilityToken(tampered, trustedIssuers, peers[1], peers[0], NOW),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("refuses issuance when the issuer key does not derive claims.iss", async () => {
    const issuer = peerOf(ISSUER_SEED);
    const subject = peerOf(new Uint8Array(32).fill(7));
    const audience = peerOf(new Uint8Array(32).fill(8));
    // Sign with a different key than the named issuer: the issuance API
    // must refuse before any bytes are signed.
    await expect(
      issueCapabilityToken(
        OTHER_ISSUER_SEED,
        claims(issuer, subject, audience, ["spoke-baseline"], NOW + 3600),
      ),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("rejects a wrong token version", async () => {
    const [proof, trustedIssuers, peers] = await happyToken(NOW, [
      "spoke-baseline",
    ]);
    const wrongVersion: CapabilityTokenProof = { ...proof, v: 2 };
    await expect(
      verifyCapabilityToken(
        wrongVersion,
        trustedIssuers,
        peers[1],
        peers[0],
        NOW,
      ),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("accepts iat at the clock-skew boundary and rejects beyond it", async () => {
    const issuer = peerOf(ISSUER_SEED);
    const subject = peerOf(new Uint8Array(32).fill(7));
    const audience = peerOf(new Uint8Array(32).fill(8));
    const trustedIssuers = trusted([issuer]);

    // iat exactly at the skew boundary is accepted.
    const atBoundary: CapabilityClaims = claims(
      issuer,
      subject,
      audience,
      ["spoke-baseline"],
      NOW + 3600,
    );
    atBoundary.iat = NOW + CLOCK_SKEW_SECONDS;
    const proofAtBoundary = await issueCapabilityToken(ISSUER_SEED, atBoundary);
    await expect(
      verifyCapabilityToken(
        proofAtBoundary,
        trustedIssuers,
        audience,
        subject,
        NOW,
      ),
    ).resolves.toEqual(["spoke-baseline"]);

    // iat beyond the skew window is rejected (issuer clock too far ahead).
    const beyond: CapabilityClaims = claims(
      issuer,
      subject,
      audience,
      ["spoke-baseline"],
      NOW + 3600,
    );
    beyond.iat = NOW + CLOCK_SKEW_SECONDS + 1;
    const proofBeyond = await issueCapabilityToken(ISSUER_SEED, beyond);
    await expect(
      verifyCapabilityToken(
        proofBeyond,
        trustedIssuers,
        audience,
        subject,
        NOW,
      ),
    ).rejects.toThrowError(tokenInvalid);

    // iat in the past is always accepted.
    const past: CapabilityClaims = claims(
      issuer,
      subject,
      audience,
      ["spoke-baseline"],
      NOW + 3600,
    );
    past.iat = NOW - 10;
    const proofPast = await issueCapabilityToken(ISSUER_SEED, past);
    await expect(
      verifyCapabilityToken(proofPast, trustedIssuers, audience, subject, NOW),
    ).resolves.toEqual(["spoke-baseline"]);
  });

  it("accepts a non-empty jti and rejects an empty jti", async () => {
    const issuer = peerOf(ISSUER_SEED);
    const subject = peerOf(new Uint8Array(32).fill(7));
    const audience = peerOf(new Uint8Array(32).fill(8));
    const trustedIssuers = trusted([issuer]);

    const withJti: CapabilityClaims = claims(
      issuer,
      subject,
      audience,
      ["spoke-baseline"],
      NOW + 3600,
    );
    withJti.jti = "token-abc-123";
    const proofWithJti = await issueCapabilityToken(ISSUER_SEED, withJti);
    await expect(
      verifyCapabilityToken(
        proofWithJti,
        trustedIssuers,
        audience,
        subject,
        NOW,
      ),
    ).resolves.toEqual(["spoke-baseline"]);

    const emptyJti: CapabilityClaims = claims(
      issuer,
      subject,
      audience,
      ["spoke-baseline"],
      NOW + 3600,
    );
    emptyJti.jti = "";
    const proofEmptyJti = await issueCapabilityToken(ISSUER_SEED, emptyJti);
    await expect(
      verifyCapabilityToken(
        proofEmptyJti,
        trustedIssuers,
        audience,
        subject,
        NOW,
      ),
    ).rejects.toThrowError(tokenInvalid);
  });

  it("produces the wire wrapper shape {v, claims, sig} with the normative claim set", async () => {
    const [proof] = await happyToken(NOW, ["spoke-baseline"]);
    expect(proof.v).toBe(1);
    expect(proof.sig).not.toBe("");
    expect(Object.keys(proof).sort()).toEqual(["claims", "sig", "v"]);
    const claimKeys = Object.keys(proof.claims);
    expect(claimKeys).toContain("iss");
    expect(claimKeys).toContain("sub");
    expect(claimKeys).toContain("aud");
    expect(claimKeys).toContain("capabilities");
    expect(claimKeys).toContain("exp");
    // Optional claims absent unless provided (omit, never `null`).
    expect(proof.claims.iat).toBeUndefined();
    expect(proof.claims.jti).toBeUndefined();
  });

  it("exposes the token constants mirroring Rust", () => {
    expect(TOKEN_VERSION).toBe(1);
    expect(CLOCK_SKEW_SECONDS).toBe(60);
  });
});

describe("verifyCapabilityToken proof-shape guard (fail closed before crypto)", () => {
  it("rejects missing or malformed proof fields with CoreError token_invalid", async () => {
    const [proof, trustedIssuers, peers] = await happyToken(NOW, ["spoke-baseline"]);

    // Runtime-shape violations a wire-parsed OpaqueJson can carry. Each must
    // fail with the CoreError `token_invalid` code — never a TypeError from
    // a downstream library (e.g. spreading a missing `capabilities`).
    const malformed: Array<[string, unknown]> = [
      ["proof is null", null],
      ["missing v", { ...proof, v: undefined }],
      ["missing sig", { ...proof, sig: undefined }],
      ["missing claims", { ...proof, claims: undefined }],
      ["claims is not an object", { ...proof, claims: "not-an-object" }],
      ["claims missing iss", { ...proof, claims: { ...proof.claims, iss: undefined } }],
      ["claims missing sub", { ...proof, claims: { ...proof.claims, sub: undefined } }],
      ["claims missing aud", { ...proof, claims: { ...proof.claims, aud: undefined } }],
      [
        "claims missing capabilities",
        { ...proof, claims: { ...proof.claims, capabilities: undefined } },
      ],
      [
        "capabilities not an array",
        { ...proof, claims: { ...proof.claims, capabilities: "spoke-baseline" } },
      ],
      [
        "capabilities contains a non-string",
        { ...proof, claims: { ...proof.claims, capabilities: ["spoke-baseline", 42] } },
      ],
      ["claims missing exp", { ...proof, claims: { ...proof.claims, exp: undefined } }],
      ["exp is a string", { ...proof, claims: { ...proof.claims, exp: "123" } }],
      ["iat is a string", { ...proof, claims: { ...proof.claims, iat: "999" } }],
      ["unknown claim key", { ...proof, claims: { ...proof.claims, extra_claim: "sneaky" } }],
      ["unknown wrapper key", { ...proof, extra_wrapper: true }],
    ];

    for (const [label, value] of malformed) {
      await expect(
        verifyCapabilityToken(
          value as CapabilityTokenProof,
          trustedIssuers,
          peers[1],
          peers[0],
          NOW,
        ),
      ).rejects.toThrowError(tokenInvalid);
    }
  });

  it("still accepts a well-formed proof (the guard does not over-reject)", async () => {
    const [proof, trustedIssuers, peers] = await happyToken(NOW, ["spoke-baseline"]);
    await expect(
      verifyCapabilityToken(proof, trustedIssuers, peers[1], peers[0], NOW),
    ).resolves.toEqual(["spoke-baseline"]);
  });
});

describe("ed25519PubkeyFromPeerId (port of peer_id.rs reverse)", () => {
  it("decodes the golden peer id back to the golden public key", () => {
    expect(ed25519PubkeyFromPeerId(GOLDEN_PEER_ID)).toEqual(GOLDEN_PUBKEY);
  });

  it("inverts derivePeerIdFromEd25519Pubkey for arbitrary keys", () => {
    for (const seedByte of [4, 5, 6]) {
      const pubkey = new Uint8Array(32).fill(seedByte);
      const peerId = derivePeerIdFromEd25519Pubkey(pubkey);
      expect(ed25519PubkeyFromPeerId(peerId)).toEqual(pubkey);
    }
  });

  it("returns undefined for non-Ed25519 peer id shapes (fail closed)", () => {
    expect(ed25519PubkeyFromPeerId("QmYwAPJzv5CZsnAzt8auVZRnV")).toBeUndefined();
    expect(ed25519PubkeyFromPeerId("12D3KooW")).toBeUndefined();
    expect(ed25519PubkeyFromPeerId("")).toBeUndefined();
    // Out-of-alphabet base58 chars must not decode.
    expect(ed25519PubkeyFromPeerId("12O3")).toBeUndefined();
  });
});
