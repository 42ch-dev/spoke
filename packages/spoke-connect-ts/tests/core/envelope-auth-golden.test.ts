import { describe, expect, it } from "vitest";

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import canonicalize from "canonicalize";

import type { ConnectInvokeResponse } from "@42ch/spoke-schemas";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  ALGORITHM_INVOKE_REQUEST_JCS_V1,
  ALGORITHM_INVOKE_RESPONSE_JCS_V1,
  ALGORITHM_SESSION_JCS_V1,
  authenticateInvokeRequest,
  authenticateInvokeResponse,
  authenticateSession,
  verifyInvokeRequestAuth,
  verifyInvokeResponseAuth,
  verifySessionAuth,
} from "../../src/core/envelope-auth.js";
import type {
  EnvelopeAuthErrorKind,
  InvokeRequestSignInput,
  InvokeResponseSignInput,
  SessionSignInput,
} from "../../src/core/envelope-auth.js";
import { fromHex, toHex } from "../hex.js";

/**
 * Cross-language golden vectors for envelope authentication: wire envelopes
 * signed by the **Rust** core (`crates/spoke-connect`, the capture
 * authority) from the identity golden seed (bytes 1..=32 — the same key
 * pair as `golden-hello.json`), checked in as
 * `tests/fixtures/golden-envelope-auth.json` (byte-identical registered
 * copy of the SSOT under `crates/spoke-connect/tests/fixtures/`; sync gate:
 * `tooling/connect/golden-vector-sync.mjs`).
 *
 * Verifying the Rust-produced envelopes end-to-end on TS pins the JCS bytes
 * TS canonicalizes from the wire fields to exactly what Rust signed, and
 * the base64url sig to the encoding Rust emits. Re-signing the same inputs
 * on TS and asserting the pinned signature reproduces byte-for-byte closes
 * the loop in the opposite direction — TS↔Rust canonical-bytes parity for
 * `spoke-connect-session-jcs-v1` /
 * `spoke-connect-invoke-request-jcs-v1` /
 * `spoke-connect-invoke-response-jcs-v1`. The negative vectors pin the
 * fail-closed rejection kinds both languages must agree on.
 */

interface PositiveVector {
  envelope_json: string;
  jcs_hex: string;
  signature_b64u: string;
}

interface NegativeVector {
  envelope_json: string;
  kind: EnvelopeAuthErrorKind;
}

interface Family {
  positive: PositiveVector[];
  negative: NegativeVector[];
}

interface GoldenEnvelopeAuth {
  version: number;
  seed_hex: string;
  pubkey_hex: string;
  peer_id: string;
  session: Family;
  invoke_request: Family;
  invoke_response: Family;
}

const fixtureUrl = new URL(
  "../../tests/fixtures/golden-envelope-auth.json",
  import.meta.url,
);
const golden = JSON.parse(
  readFileSync(fileURLToPath(fixtureUrl), "utf8"),
) as GoldenEnvelopeAuth;

const GOLDEN_SEED = fromHex(golden.seed_hex);
const GOLDEN_PUBKEY = fromHex(golden.pubkey_hex);

/** Parse a pinned wire envelope (the form the verify helpers consume). */
function wireOf(envelopeJson: string): Record<string, unknown> {
  return JSON.parse(envelopeJson) as Record<string, unknown>;
}

/** The signed object a session envelope covers: wire minus signature/extensions. */
function sessionSignInput(wire: Record<string, unknown>): SessionSignInput {
  return {
    session_id: wire.session_id as string,
    initiator_peer_id: wire.initiator_peer_id as string,
    responder_peer_id: wire.responder_peer_id as string,
    opened_at: wire.opened_at as string,
    negotiated_capabilities: wire.negotiated_capabilities as [string, ...string[]],
    initial_sequence: wire.initial_sequence as 0,
  };
}

function invokeRequestSignInput(wire: Record<string, unknown>): InvokeRequestSignInput {
  const input: InvokeRequestSignInput = {
    session_id: wire.session_id as string,
    sequence: wire.sequence as number,
    request_id: wire.request_id as string,
    op: wire.op as string,
    payload: wire.payload as { [k: string]: unknown },
  };
  if ("auth" in wire) {
    input.auth = wire.auth as Record<string, unknown> | undefined;
  }
  return input;
}

function invokeResponseSignInput(wire: Record<string, unknown>): InvokeResponseSignInput {
  const base = {
    session_id: wire.session_id as string,
    sequence: wire.sequence as number,
    request_id: wire.request_id as string,
  };
  if ("payload" in wire) {
    return { ...base, payload: wire.payload as { [k: string]: unknown } };
  }
  return {
    ...base,
    error: wire.error as Extract<ConnectInvokeResponse, { error: object }>["error"],
  };
}

/** JCS-canonicalize a signed object and return its UTF-8 bytes as hex. */
function jcsHex(signedObject: Record<string, unknown>): string {
  const jcs = canonicalize(signedObject);
  if (jcs === undefined) {
    throw new Error("canonicalize returned undefined — signed object is not JSON-serializable");
  }
  return toHex(new TextEncoder().encode(jcs));
}

/** Assert a verify call rejects with exactly the locked machine kind. */
async function rejectsWithKind(
  promise: Promise<unknown>,
  kind: EnvelopeAuthErrorKind,
): Promise<void> {
  await expect(promise).rejects.toMatchObject({ kind });
}

describe("Rust-minted envelope-auth golden vectors", () => {
  it("derives the golden key story and pins every positive signature", () => {
    expect(golden.version).toBe(1);
    expect(golden.seed_hex).toMatch(/^[0-9a-f]{64}$/);
    expect(golden.pubkey_hex).toMatch(/^[0-9a-f]{64}$/);
    expect(derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(GOLDEN_SEED))).toBe(
      golden.peer_id,
    );
    expect(toHex(getPublicKeyEd25519(GOLDEN_SEED))).toBe(golden.pubkey_hex);

    // The golden host (the fixture peer_id) signs its own session snapshot.
    const sessionWire = wireOf(golden.session.positive[0].envelope_json);
    expect(sessionWire.initiator_peer_id).toBe(golden.peer_id);

    for (const family of [
      golden.session,
      golden.invoke_request,
      golden.invoke_response,
    ]) {
      expect(family.positive.length).toBeGreaterThanOrEqual(1);
      expect(family.negative.length).toBeGreaterThanOrEqual(1);
      for (const positive of family.positive) {
        expect(wireOf(positive.envelope_json).signature).toBe(positive.signature_b64u);
        // 64 raw signature bytes → 86 base64url chars, no padding.
        expect(positive.signature_b64u).toHaveLength(86);
      }
    }
    // The invoke-response family pins both branches (never merged).
    const responsePositives = golden.invoke_response.positive;
    expect(responsePositives.length).toBe(2);
    expect(wireOf(responsePositives[0].envelope_json)).toHaveProperty("payload");
    expect(wireOf(responsePositives[1].envelope_json)).toHaveProperty("error");
  });

  it("session: positive verifies and re-signs byte-identical", async () => {
    const wire = wireOf(golden.session.positive[0].envelope_json);
    const positive = golden.session.positive[0];

    await expect(
      verifySessionAuth(
        GOLDEN_PUBKEY,
        wire as unknown as Parameters<typeof verifySessionAuth>[1],
        wire.initiator_peer_id as string,
        wire.responder_peer_id as string,
      ),
    ).resolves.toBeUndefined();

    // Re-sign the received wire fields: identical JCS bytes + signature.
    const input = sessionSignInput(wire);
    expect(jcsHex(input as unknown as Record<string, unknown>)).toBe(positive.jcs_hex);
    const reSigned = await authenticateSession(GOLDEN_SEED, input);
    expect(reSigned.signature).toBe(positive.signature_b64u);
  });

  it("session: negative rejects with the pinned kind", async () => {
    for (const negative of golden.session.negative) {
      const wire = wireOf(negative.envelope_json);
      await rejectsWithKind(
        verifySessionAuth(
          GOLDEN_PUBKEY,
          wire as unknown as Parameters<typeof verifySessionAuth>[1],
          wire.initiator_peer_id as string,
          wire.responder_peer_id as string,
        ),
        negative.kind,
      );
    }
  });

  it("invoke-request: positive (with auth) verifies and re-signs byte-identical", async () => {
    const wire = wireOf(golden.invoke_request.positive[0].envelope_json);
    const positive = golden.invoke_request.positive[0];

    await expect(
      verifyInvokeRequestAuth(
        GOLDEN_PUBKEY,
        wire as unknown as Parameters<typeof verifyInvokeRequestAuth>[1],
        wire.session_id as string,
      ),
    ).resolves.toBeUndefined();

    const input = invokeRequestSignInput(wire);
    // The pinned vector carries `auth` — conditional inclusion must survive
    // the re-sign (a lost auth would change the signed bytes).
    expect(input.auth).toBeDefined();
    expect(jcsHex(input as unknown as Record<string, unknown>)).toBe(positive.jcs_hex);
    const reSigned = await authenticateInvokeRequest(GOLDEN_SEED, input);
    expect(reSigned.signature).toBe(positive.signature_b64u);
  });

  it("invoke-request: negative rejects with the pinned kind", async () => {
    for (const negative of golden.invoke_request.negative) {
      const wire = wireOf(negative.envelope_json);
      await rejectsWithKind(
        verifyInvokeRequestAuth(
          GOLDEN_PUBKEY,
          wire as unknown as Parameters<typeof verifyInvokeRequestAuth>[1],
          wire.session_id as string,
        ),
        negative.kind,
      );
    }
  });

  it("invoke-response: both branches verify and re-sign byte-identical", async () => {
    for (const positive of golden.invoke_response.positive) {
      const wire = wireOf(positive.envelope_json);

      await expect(
        verifyInvokeResponseAuth(
          GOLDEN_PUBKEY,
          wire as unknown as Parameters<typeof verifyInvokeResponseAuth>[1],
          wire.session_id as string,
        ),
      ).resolves.toBeUndefined();

      const input = invokeResponseSignInput(wire);
      expect(jcsHex(input as unknown as Record<string, unknown>)).toBe(positive.jcs_hex);
      const reSigned = await authenticateInvokeResponse(GOLDEN_SEED, input);
      expect(reSigned.signature).toBe(positive.signature_b64u);
    }
  });

  it("invoke-response: negative rejects with the pinned kind", async () => {
    for (const negative of golden.invoke_response.negative) {
      const wire = wireOf(negative.envelope_json);
      await rejectsWithKind(
        verifyInvokeResponseAuth(
          GOLDEN_PUBKEY,
          wire as unknown as Parameters<typeof verifyInvokeResponseAuth>[1],
          wire.session_id as string,
        ),
        negative.kind,
      );
    }
  });
});
