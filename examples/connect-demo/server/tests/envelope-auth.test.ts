/**
 * Envelope-auth unit tests (protocol_version 2) — spec-derived sign/verify
 * over the locked field sets and algorithm ids
 * (`.mstar/specs/spoke-connect.md` §Envelope authentication):
 *
 * - `spoke-connect-session-jcs-v1`: signed object =
 *   `{session_id, initiator_peer_id, responder_peer_id, opened_at,
 *   negotiated_capabilities, initial_sequence}`;
 * - `spoke-connect-invoke-request-jcs-v1`: signed object =
 *   `{session_id, sequence, request_id, op, payload}` plus `auth` WHEN
 *   present on the wire (conditional inclusion);
 * - `spoke-connect-invoke-response-jcs-v1`: signed object mirrors the wire
 *   branch exactly — `{session_id, sequence, request_id, payload}` (success)
 *   XOR `{session_id, sequence, request_id, error}` (error), never merged.
 *
 * The demo module SIGNS session snapshots + invoke responses and VERIFIES
 * inbound invoke requests (responder role). The reverse directions are
 * cross-checked with an independent helper below built from the same public
 * primitives, so both directions are proven against the spec field sets.
 */

import { describe, expect, it } from "vitest";

import canonicalize from "canonicalize";

import type {
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ConnectSession,
  ErrorEnvelope,
} from "@42ch/spoke-schemas";
import {
  base64UrlDecode,
  base64UrlEncode,
  getPublicKeyEd25519,
  signEd25519,
  verifyEd25519,
} from "@42ch/spoke-connect";

import {
  createEnvelopeAuth,
  EnvelopeAuthError,
  type EnvelopeAuthErrorKind,
} from "../src/host/envelope-auth.js";
import {
  DEMO_CLIENT_PEER_ID,
  DEMO_SERVER_PEER_ID,
  DEMO_SERVER_PUBKEY,
  DEMO_SERVER_SEED,
  DEMO_STRANGER_PUBKEY,
} from "../src/identities.js";

const textEncoder = new TextEncoder();

// ── fixtures ────────────────────────────────────────────────────────────────

const SESSION_ID = "demo-session-envelope-test-0001";

const SESSION_INPUT = {
  session_id: SESSION_ID,
  initiator_peer_id: DEMO_CLIENT_PEER_ID,
  responder_peer_id: DEMO_SERVER_PEER_ID,
  opened_at: "2026-08-06T00:00:00.000Z",
  negotiated_capabilities: ["spoke-baseline"] as [string, ...string[]],
  initial_sequence: 0 as const,
};

const REQUEST_BASE = {
  session_id: SESSION_ID,
  sequence: 0,
  request_id: "demo-request-0001",
  op: "port.knowledge.get",
  payload: { entry_id: "demo-harbor/character/mira" },
};

/** Opaque `auth` blob — any object; it is trust-affecting and MUST be bound. */
const AUTH_BLOB = {
  v: 1,
  claims: { iss: DEMO_SERVER_PEER_ID, capabilities: ["spoke-baseline"] },
  sig: "opaque-proof-blob",
};

const ERROR_ENVELOPE: ErrorEnvelope = {
  code: "KNOWLEDGE_ENTRY_NOT_FOUND",
  message: "KnowledgeEntry not found",
  details: { entry_id: "demo-harbor/character/ghost" },
  extensions: {},
};

// ── independent test-side primitives (spec field sets, public crypto) ──────

/** JCS-canonicalize to UTF-8 bytes (RFC 8785, same construction as hello). */
function jcsBytes(obj: unknown): Uint8Array {
  const jcs = canonicalize(obj);
  if (jcs === undefined) {
    throw new Error("signed object is not JSON-serializable");
  }
  return textEncoder.encode(jcs);
}

/** Sign a signed object with a raw Ed25519 seed → canonical base64url. */
async function signEnvelopeTest(
  seed: Uint8Array,
  signedObject: Record<string, unknown>,
): Promise<string> {
  return base64UrlEncode(await signEd25519(seed, jcsBytes(signedObject)));
}

/** The locked `ConnectSession` signed object (exactly six keys). */
function sessionSignedObject(wire: Record<string, unknown>): Record<string, unknown> {
  return {
    session_id: wire.session_id,
    initiator_peer_id: wire.initiator_peer_id,
    responder_peer_id: wire.responder_peer_id,
    opened_at: wire.opened_at,
    negotiated_capabilities: wire.negotiated_capabilities,
    initial_sequence: wire.initial_sequence,
  };
}

/** The locked `ConnectInvokeRequest` signed object (`auth` conditional). */
function requestSignedObject(wire: Record<string, unknown>): Record<string, unknown> {
  const signedObject: Record<string, unknown> = {
    session_id: wire.session_id,
    sequence: wire.sequence,
    request_id: wire.request_id,
    op: wire.op,
    payload: wire.payload,
  };
  if (wire.auth !== undefined) {
    signedObject.auth = wire.auth;
  }
  return signedObject;
}

/** The locked `ConnectInvokeResponse` signed object (branch-exact, never merged). */
function responseSignedObject(wire: Record<string, unknown>): Record<string, unknown> {
  const hasPayload = "payload" in wire;
  const hasError = "error" in wire;
  if (hasPayload === hasError) {
    throw new Error("response must carry exactly one of payload or error");
  }
  return {
    session_id: wire.session_id,
    sequence: wire.sequence,
    request_id: wire.request_id,
    ...(hasPayload ? { payload: wire.payload } : { error: wire.error }),
  };
}

/**
 * Independent verify over the locked verify steps: presence → canonical
 * base64url round-trip → 64-byte signature → JCS → Ed25519 with the public
 * key that derives the signer's peer_id.
 */
async function verifyEnvelopeTest(
  pubkey: Uint8Array,
  wire: unknown,
  signedObject: (wire: Record<string, unknown>) => Record<string, unknown>,
): Promise<void> {
  const wireRecord = wire as Record<string, unknown>;
  const signatureField = wireRecord.signature;
  if (typeof signatureField !== "string" || signatureField === "") {
    throw new Error("envelope is missing a signature");
  }
  const signature = base64UrlDecode(signatureField);
  if (base64UrlEncode(signature) !== signatureField) {
    throw new Error("signature is not canonical base64url");
  }
  if (signature.length !== 64) {
    throw new Error("signature is not 64 bytes");
  }
  if (
    !(await verifyEd25519(pubkey, jcsBytes(signedObject(wireRecord)), signature))
  ) {
    throw new Error("signature does not verify");
  }
}

/** Sign a full wire `ConnectInvokeRequest` over the locked field set. */
async function signRequestTest(
  seed: Uint8Array,
  request: {
    session_id: string;
    sequence: number;
    request_id: string;
    op: string;
    payload: Record<string, unknown>;
    auth?: Record<string, unknown>;
  },
): Promise<ConnectInvokeRequest> {
  const signature = await signEnvelopeTest(
    seed,
    requestSignedObject({ ...request }),
  );
  return { ...request, signature, extensions: {} };
}

// ── suite ───────────────────────────────────────────────────────────────────

const auth = createEnvelopeAuth({ seed: DEMO_SERVER_SEED });

/** Run `verifyInvokeRequestAuth`, asserting the locked rejection kind. */
async function expectAuthError(
  request: ConnectInvokeRequest,
  kind: EnvelopeAuthErrorKind,
): Promise<void> {
  const error = await auth
    .verifyInvokeRequestAuth(DEMO_SERVER_PUBKEY, request, SESSION_ID)
    .then(
      () => null,
      (caught: unknown) => caught,
    );
  expect(error).toBeInstanceOf(EnvelopeAuthError);
  expect((error as EnvelopeAuthError).kind).toBe(kind);
  expect((error as EnvelopeAuthError).details.kind).toBe(kind);
}

describe("createEnvelopeAuth — session snapshot (spoke-connect-session-jcs-v1)", () => {
  it("signs the locked 6-field set as a canonical 86-char base64url signature", async () => {
    const wire: ConnectSession = await auth.authenticateSession(SESSION_INPUT);

    expect(wire.signature).toHaveLength(86);
    // Canonical encoding (parity-binding): decode → encode round-trips.
    expect(base64UrlEncode(base64UrlDecode(wire.signature))).toBe(wire.signature);
    expect(wire.extensions).toEqual({});

    // Known-answer cross-check: Ed25519 is deterministic, so signing the
    // same spec field-set object with the same seed reproduces the exact
    // signature the module emitted.
    const expected = await signEnvelopeTest(
      DEMO_SERVER_SEED,
      sessionSignedObject({ ...SESSION_INPUT }),
    );
    expect(wire.signature).toBe(expected);
  });

  it("verifies against the public key derived from the seed (self-consistency)", async () => {
    const wire: ConnectSession = await auth.authenticateSession(SESSION_INPUT);
    await expect(
      verifyEnvelopeTest(DEMO_SERVER_PUBKEY, wire, sessionSignedObject),
    ).resolves.toBeUndefined();
  });
});

describe("createEnvelopeAuth — invoke request verify (spoke-connect-invoke-request-jcs-v1)", () => {
  it("verifies a request signed WITHOUT auth (5-field signed object)", async () => {
    const wire = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    expect(wire.signature).toHaveLength(86);
    await expect(
      auth.verifyInvokeRequestAuth(DEMO_SERVER_PUBKEY, wire, SESSION_ID),
    ).resolves.toBeUndefined();
  });

  it("verifies a request signed WITH auth (6-field signed object, conditional inclusion)", async () => {
    const wire = await signRequestTest(DEMO_SERVER_SEED, {
      ...REQUEST_BASE,
      auth: AUTH_BLOB,
    });
    expect(wire.auth).toEqual(AUTH_BLOB);
    expect(wire.signature).toHaveLength(86);
    await expect(
      auth.verifyInvokeRequestAuth(DEMO_SERVER_PUBKEY, wire, SESSION_ID),
    ).resolves.toBeUndefined();
  });

  it("rejects a request whose wire auth was NOT covered by the signature", async () => {
    // Signed over the 5-field object (auth absent), auth added to the wire
    // afterwards: the verify builds the 6-field object, so the bytes differ.
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    await expectAuthError({ ...signed, auth: AUTH_BLOB }, "envelope_auth_invalid");
  });

  it("rejects a request signed with auth but auth stripped from the wire", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, {
      ...REQUEST_BASE,
      auth: AUTH_BLOB,
    });
    const { auth: _auth, ...stripped } = signed;
    await expectAuthError(stripped, "envelope_auth_invalid");
  });
});

describe("createEnvelopeAuth — invoke response signing (spoke-connect-invoke-response-jcs-v1)", () => {
  it("signs the success branch over {session_id, sequence, request_id, payload}", async () => {
    const wire: ConnectInvokeResponse = await auth.authenticateInvokeResponse({
      session_id: SESSION_ID,
      sequence: 0,
      request_id: REQUEST_BASE.request_id,
      payload: { ok: true },
    });

    expect(wire.signature).toHaveLength(86);
    expect(base64UrlEncode(base64UrlDecode(wire.signature))).toBe(wire.signature);
    await expect(
      verifyEnvelopeTest(DEMO_SERVER_PUBKEY, wire, responseSignedObject),
    ).resolves.toBeUndefined();
  });

  it("signs the error branch over {session_id, sequence, request_id, error}", async () => {
    const wire: ConnectInvokeResponse = await auth.authenticateInvokeResponse({
      session_id: SESSION_ID,
      sequence: 0,
      request_id: REQUEST_BASE.request_id,
      error: ERROR_ENVELOPE,
    });

    expect(wire.signature).toHaveLength(86);
    await expect(
      verifyEnvelopeTest(DEMO_SERVER_PUBKEY, wire, responseSignedObject),
    ).resolves.toBeUndefined();
  });

  it("never merges the two branches: a success signature fails over a payload+error object", async () => {
    const success = await auth.authenticateInvokeResponse({
      session_id: SESSION_ID,
      sequence: 0,
      request_id: REQUEST_BASE.request_id,
      payload: { ok: true },
    });
    // The independent verifier enforces the branch-exact signed object
    // (payload XOR error), mirroring the spec's never-merged rule.
    const merged = {
      ...success,
      error: ERROR_ENVELOPE,
    } as unknown as Record<string, unknown>;
    await expect(
      verifyEnvelopeTest(DEMO_SERVER_PUBKEY, merged, responseSignedObject),
    ).rejects.toThrow("exactly one of payload or error");
  });
});

describe("verifyInvokeRequestAuth — fail-closed tamper rejections (locked details.kind)", () => {
  it("rejects a stripped signature as envelope_auth_missing", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    await expectAuthError({ ...signed, signature: "" }, "envelope_auth_missing");

    const noSignature = { ...signed } as ConnectInvokeRequest;
    delete (noSignature as Partial<ConnectInvokeRequest>).signature;
    await expectAuthError(noSignature, "envelope_auth_missing");
  });

  it("rejects a signature verified with the wrong key as envelope_auth_invalid", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    const error = await auth
      .verifyInvokeRequestAuth(DEMO_STRANGER_PUBKEY, signed, SESSION_ID)
      .then(
        () => null,
        (caught: unknown) => caught,
      );
    expect(error).toBeInstanceOf(EnvelopeAuthError);
    expect((error as EnvelopeAuthError).kind).toBe("envelope_auth_invalid");
  });

  it("rejects a mutated signed field as envelope_auth_invalid", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    await expectAuthError(
      { ...signed, payload: { entry_id: "demo-harbor/character/ghost" } },
      "envelope_auth_invalid",
    );
  });

  it("rejects non-canonical base64url as envelope_auth_invalid", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    // Padded input: decode → encode strips the padding, so the round-trip
    // equality check rejects it.
    await expectAuthError(
      { ...signed, signature: `${signed.signature}=` },
      "envelope_auth_invalid",
    );
    // Alternate slack-bit encoding of the final character: changing the
    // last base64url character changes the decoded bytes, so re-encoding
    // does not reproduce the wire value.
    const slackVariant =
      signed.signature.slice(0, -1) +
      (signed.signature.endsWith("A") ? "B" : "A");
    await expectAuthError(
      { ...signed, signature: slackVariant },
      "envelope_auth_invalid",
    );
  });

  it("rejects a non-64-byte signature as envelope_auth_invalid", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    const shortSignature = base64UrlEncode(new Uint8Array(32).fill(7));
    expect(shortSignature).toHaveLength(43);
    await expectAuthError(
      { ...signed, signature: shortSignature },
      "envelope_auth_invalid",
    );
  });

  it("rejects unknown wire keys (field-set drift) as envelope_auth_invalid", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    const drift = { ...signed, evil: "field-set drift" } as ConnectInvokeRequest;
    await expectAuthError(drift, "envelope_auth_invalid");
  });

  it("rejects a missing signed field as envelope_auth_invalid", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    const missingOp = { ...signed } as ConnectInvokeRequest;
    delete (missingOp as Partial<ConnectInvokeRequest>).op;
    await expectAuthError(missingOp, "envelope_auth_invalid");
  });

  it("rejects a valid signature bound to another session as envelope_auth_session_unbound", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    const error = await auth
      .verifyInvokeRequestAuth(
        DEMO_SERVER_PUBKEY,
        signed,
        "demo-session-other-0002",
      )
      .then(
        () => null,
        (caught: unknown) => caught,
      );
    expect(error).toBeInstanceOf(EnvelopeAuthError);
    expect((error as EnvelopeAuthError).kind).toBe("envelope_auth_session_unbound");
  });
});

describe("createEnvelopeAuth — key misuse is a loud local error, not a wire rejection", () => {
  it("throws on a wrong-length seed at sign time", async () => {
    const bad = createEnvelopeAuth({ seed: new Uint8Array(16) });
    await expect(bad.authenticateSession(SESSION_INPUT)).rejects.toThrow(
      "secret must be 32 bytes",
    );
  });

  it("throws (not EnvelopeAuthError) on a wrong-length public key at verify time", async () => {
    const signed = await signRequestTest(DEMO_SERVER_SEED, REQUEST_BASE);
    await expect(
      auth.verifyInvokeRequestAuth(
        new Uint8Array(16),
        signed,
        SESSION_ID,
      ),
    ).rejects.toThrow("public key must be 32 bytes");
  });
});
