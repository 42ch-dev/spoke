import { describe, expect, it } from "vitest";

import type {
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ConnectSession,
} from "@42ch/spoke-schemas";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  ALGORITHM_INVOKE_REQUEST_JCS_V1,
  ALGORITHM_INVOKE_RESPONSE_JCS_V1,
  ALGORITHM_SESSION_JCS_V1,
  EnvelopeAuthError,
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

/**
 * Envelope-auth core tests — mirror the locked contract
 * (`.mstar/specs/spoke-connect.md` §Envelope authentication):
 * algorithm ids, exact signed-field sets, conditional-`auth` inclusion,
 * oneOf invoke-response branch separation, canonical base64url round-trip,
 * and the fail-closed verify order with stable `details.kind` values.
 *
 * Deterministic seeds per role (Rust unit tests use the same convention):
 * the envelope emitter is `[7u8; 32]`; a distinct key `[8u8; 32]` stands in
 * for a wrong peer / tamper key; the two session peers are `[9u8; 32]` and
 * `[10u8; 32]`.
 */

const EMITTER_SEED = new Uint8Array(32).fill(7);
const EMITTER_PUBKEY = getPublicKeyEd25519(EMITTER_SEED);
const OTHER_SEED = new Uint8Array(32).fill(8);
const OTHER_PUBKEY = getPublicKeyEd25519(OTHER_SEED);
const INITIATOR_SEED = new Uint8Array(32).fill(9);
const RESPONDER_SEED = new Uint8Array(32).fill(10);

const INITIATOR_PEER_ID = peerOf(INITIATOR_SEED);
const RESPONDER_PEER_ID = peerOf(RESPONDER_SEED);
const SESSION_ID = "session-0001";

/** A peer id for a raw Ed25519 seed (runtime-derived — no literals in crypto positions). */
function peerOf(seed: Uint8Array): string {
  return derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed));
}

/**
 * Cast an OpaqueJson `auth` value (any JSON — scalar, null, array, object)
 * through the generated schema type, whose `auth` is object-only. The
 * contract (§3) types `auth` as `<OpaqueJson>` and both runtimes bind
 * arbitrary JSON (Rust `Option<serde_json::Value>`), so the generated type
 * is narrower than the contract — the cast is a type-level accommodation
 * only (schema codegen nit tracked as QC F-5; the schema itself is not
 * widened in this wave).
 */
function opaqueAuth(auth: unknown): ConnectInvokeRequest["auth"] {
  return auth as ConnectInvokeRequest["auth"];
}

/** Assert a verify call rejects with exactly the locked machine kind. */
async function rejectsWithKind(
  promise: Promise<unknown>,
  kind: EnvelopeAuthErrorKind,
): Promise<void> {
  await expect(promise).rejects.toMatchObject({ kind });
}

/** Swap the final base64url char for a sibling encoding the same 64 bytes with non-zero slack bits. */
function nonCanonicalSignature(encoded: string): string {
  const last = encoded[encoded.length - 1];
  const sibling =
    ({ A: "B", Q: "R", g: "h", w: "x" } as Record<string, string>)[last] ?? "A";
  return encoded.slice(0, -1) + sibling;
}

/** The locked session signed-field set as a sign input. */
function sessionInput(): SessionSignInput {
  return {
    session_id: SESSION_ID,
    initiator_peer_id: INITIATOR_PEER_ID,
    responder_peer_id: RESPONDER_PEER_ID,
    opened_at: "2026-08-05T00:00:00Z",
    negotiated_capabilities: ["spoke-connect"],
    initial_sequence: 0,
  };
}

/** A request sign input; `extra` lets tests add/override `auth` and friends. */
function requestInput(extra: Partial<InvokeRequestSignInput> = {}): InvokeRequestSignInput {
  return {
    session_id: SESSION_ID,
    sequence: 0,
    request_id: "req-0001",
    op: "upsert",
    payload: { collection: "notes", id: "n1", value: { title: "hello" } },
    ...extra,
  };
}

/** Success-branch response sign input (oneOf branch 1). */
function responseSuccessInput(): InvokeResponseSignInput {
  return {
    session_id: SESSION_ID,
    sequence: 0,
    request_id: "req-0001",
    payload: { ok: true, result: { id: "n1" } },
  };
}

/** Error-branch response sign input (oneOf branch 2). */
function responseErrorInput(): InvokeResponseSignInput {
  return {
    session_id: SESSION_ID,
    sequence: 0,
    request_id: "req-0001",
    error: { code: "op_unsupported", message: "unknown op", extensions: {} },
  };
}

describe("algorithm ids (locked, contract §2)", () => {
  it("exposes the three locked algorithm ids verbatim", () => {
    expect(ALGORITHM_SESSION_JCS_V1).toBe("spoke-connect-session-jcs-v1");
    expect(ALGORITHM_INVOKE_REQUEST_JCS_V1).toBe("spoke-connect-invoke-request-jcs-v1");
    expect(ALGORITHM_INVOKE_RESPONSE_JCS_V1).toBe("spoke-connect-invoke-response-jcs-v1");
  });
});

describe("ConnectSession envelope auth (spoke-connect-session-jcs-v1)", () => {
  it("signs a session snapshot to an 86-char signature and verifies (happy path)", async () => {
    const session = await authenticateSession(EMITTER_SEED, sessionInput());
    expect(session.signature).toMatch(/^[A-Za-z0-9_-]{86}$/);
    expect(session.extensions).toEqual({});
    // Verify with the emitter's hello public key + the authenticated hellos.
    await verifySessionAuth(
      EMITTER_PUBKEY,
      session,
      INITIATOR_PEER_ID,
      RESPONDER_PEER_ID,
    );
  });

  it("rejects a tampered signed field (envelope_auth_invalid)", async () => {
    const session = await authenticateSession(EMITTER_SEED, sessionInput());
    const tampered = {
      ...session,
      opened_at: "2026-08-06T00:00:00Z",
    };
    await rejectsWithKind(
      verifySessionAuth(EMITTER_PUBKEY, tampered, INITIATOR_PEER_ID, RESPONDER_PEER_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a session missing its signature (envelope_auth_missing)", async () => {
    const session = await authenticateSession(EMITTER_SEED, sessionInput());
    const { signature: _signature, ...missing } = session as unknown as Record<string, unknown>;
    await rejectsWithKind(
      verifySessionAuth(
        EMITTER_PUBKEY,
        missing as unknown as ConnectSession,
        INITIATOR_PEER_ID,
        RESPONDER_PEER_ID,
      ),
      "envelope_auth_missing",
    );
  });

  it("rejects a non-canonical signature encoding (envelope_auth_invalid)", async () => {
    const session = await authenticateSession(EMITTER_SEED, sessionInput());
    const nonCanonical = { ...session, signature: nonCanonicalSignature(session.signature) };
    await rejectsWithKind(
      verifySessionAuth(EMITTER_PUBKEY, nonCanonical, INITIATOR_PEER_ID, RESPONDER_PEER_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a signature verified with the wrong public key (envelope_auth_invalid)", async () => {
    const session = await authenticateSession(EMITTER_SEED, sessionInput());
    await rejectsWithKind(
      verifySessionAuth(OTHER_PUBKEY, session, INITIATOR_PEER_ID, RESPONDER_PEER_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects peer ids that do not match the authenticated hellos (envelope_auth_session_unbound)", async () => {
    const session = await authenticateSession(EMITTER_SEED, sessionInput());
    // Signature verifies (emitter key) but the snapshot's peer ids do not
    // match the authenticated hellos of the current session.
    await rejectsWithKind(
      verifySessionAuth(EMITTER_PUBKEY, session, peerOf(OTHER_SEED), RESPONDER_PEER_ID),
      "envelope_auth_session_unbound",
    );
  });

  it("rejects an unknown wire key (field-set drift, envelope_auth_invalid)", async () => {
    const session = await authenticateSession(EMITTER_SEED, sessionInput());
    const drifted = { ...session, rogue: "field" } as unknown as ConnectSession;
    await rejectsWithKind(
      verifySessionAuth(EMITTER_PUBKEY, drifted, INITIATOR_PEER_ID, RESPONDER_PEER_ID),
      "envelope_auth_invalid",
    );
  });

  it("carries the locked machine shape: code auth_failed + details.kind", async () => {
    const session = await authenticateSession(EMITTER_SEED, sessionInput());
    const err = await verifySessionAuth(
      OTHER_PUBKEY,
      session,
      INITIATOR_PEER_ID,
      RESPONDER_PEER_ID,
    ).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(EnvelopeAuthError);
    expect((err as EnvelopeAuthError).code).toBe("auth_failed");
    expect((err as EnvelopeAuthError).kind).toBe("envelope_auth_invalid");
    expect((err as EnvelopeAuthError).details).toEqual({
      kind: "envelope_auth_invalid",
    });
  });
});

describe("ConnectInvokeRequest envelope auth (spoke-connect-invoke-request-jcs-v1)", () => {
  it("signs a request WITH auth and verifies (auth bound when present)", async () => {
    const auth = { method: "capability-token", sig: "opaque" };
    const request = await authenticateInvokeRequest(
      EMITTER_SEED,
      requestInput({ auth }),
    );
    expect(request.auth).toEqual(auth);
    expect(request.signature).toMatch(/^[A-Za-z0-9_-]{86}$/);
    await verifyInvokeRequestAuth(EMITTER_PUBKEY, request, SESSION_ID);
  });

  it("signs a request WITHOUT auth and verifies (auth absent in the signed object)", async () => {
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput());
    expect("auth" in request).toBe(false);
    await verifyInvokeRequestAuth(EMITTER_PUBKEY, request, SESSION_ID);
  });

  it("signs a request with scalar auth and verifies (OpaqueJson bound verbatim)", async () => {
    const auth = opaqueAuth("opaque-token");
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput({ auth }));
    expect(request.auth).toBe(auth);
    await verifyInvokeRequestAuth(EMITTER_PUBKEY, request, SESSION_ID);
  });

  it("signs a request with null auth and verifies (OpaqueJson bound verbatim)", async () => {
    const request = await authenticateInvokeRequest(
      EMITTER_SEED,
      requestInput({ auth: opaqueAuth(null) }),
    );
    expect(request.auth).toBeNull();
    await verifyInvokeRequestAuth(EMITTER_PUBKEY, request, SESSION_ID);
  });

  it("signs a request with array auth and verifies (OpaqueJson bound verbatim)", async () => {
    const auth = opaqueAuth(["capability-token", { sig: "opaque" }]);
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput({ auth }));
    expect(request.auth).toEqual(auth);
    await verifyInvokeRequestAuth(EMITTER_PUBKEY, request, SESSION_ID);
  });

  it("rejects a tampered scalar auth (envelope_auth_invalid)", async () => {
    const request = await authenticateInvokeRequest(
      EMITTER_SEED,
      requestInput({ auth: opaqueAuth("opaque-token") }),
    );
    const tampered = { ...request, auth: opaqueAuth("forged-token") };
    await rejectsWithKind(
      verifyInvokeRequestAuth(EMITTER_PUBKEY, tampered, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a tampered payload (envelope_auth_invalid)", async () => {
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput());
    const tampered = {
      ...request,
      payload: { collection: "notes", id: "n1", value: { title: "tampered" } },
    };
    await rejectsWithKind(
      verifyInvokeRequestAuth(EMITTER_PUBKEY, tampered, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a tampered auth field (auth is trust-affecting and bound)", async () => {
    const request = await authenticateInvokeRequest(
      EMITTER_SEED,
      requestInput({ auth: { method: "capability-token", sig: "opaque" } }),
    );
    const tampered = { ...request, auth: { method: "capability-token", sig: "forged" } };
    await rejectsWithKind(
      verifyInvokeRequestAuth(EMITTER_PUBKEY, tampered, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a request whose auth was stripped after signing (envelope_auth_invalid)", async () => {
    const request = await authenticateInvokeRequest(
      EMITTER_SEED,
      requestInput({ auth: { method: "capability-token", sig: "opaque" } }),
    );
    const stripped = { ...request } as Partial<ConnectInvokeRequest>;
    delete stripped.auth;
    await rejectsWithKind(
      verifyInvokeRequestAuth(EMITTER_PUBKEY, stripped as ConnectInvokeRequest, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a request with auth added after signing (envelope_auth_invalid)", async () => {
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput());
    const forged = { ...request, auth: { method: "capability-token", sig: "opaque" } };
    await rejectsWithKind(
      verifyInvokeRequestAuth(EMITTER_PUBKEY, forged, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a request missing its signature (envelope_auth_missing)", async () => {
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput());
    const { signature: _signature, ...missing } = request as unknown as Record<string, unknown>;
    await rejectsWithKind(
      verifyInvokeRequestAuth(
        EMITTER_PUBKEY,
        missing as unknown as ConnectInvokeRequest,
        SESSION_ID,
      ),
      "envelope_auth_missing",
    );
  });

  it("rejects a session_id not bound to the session (envelope_auth_session_unbound)", async () => {
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput());
    await rejectsWithKind(
      verifyInvokeRequestAuth(EMITTER_PUBKEY, request, "session-other"),
      "envelope_auth_session_unbound",
    );
  });

  it("rejects an unknown wire key (field-set drift, envelope_auth_invalid)", async () => {
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput());
    const drifted = { ...request, rogue: "field" } as unknown as ConnectInvokeRequest;
    await rejectsWithKind(
      verifyInvokeRequestAuth(EMITTER_PUBKEY, drifted, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a non-32-byte verify public key with CoreError crypto (local misuse, not a wire kind)", async () => {
    const request = await authenticateInvokeRequest(EMITTER_SEED, requestInput());
    const shortKey = new Uint8Array(16).fill(7);
    await expect(
      verifyInvokeRequestAuth(shortKey, request, SESSION_ID),
    ).rejects.toMatchObject({ name: "CoreError", code: "crypto" });
  });
});

describe("ConnectInvokeResponse envelope auth (spoke-connect-invoke-response-jcs-v1)", () => {
  it("signs and verifies the SUCCESS branch (payload signed object)", async () => {
    const response = await authenticateInvokeResponse(EMITTER_SEED, responseSuccessInput());
    expect("payload" in response).toBe(true);
    expect("error" in response).toBe(false);
    expect(response.signature).toMatch(/^[A-Za-z0-9_-]{86}$/);
    await verifyInvokeResponseAuth(EMITTER_PUBKEY, response, SESSION_ID);
  });

  it("signs and verifies the ERROR branch (error signed object)", async () => {
    const response = await authenticateInvokeResponse(EMITTER_SEED, responseErrorInput());
    expect("error" in response).toBe(true);
    expect("payload" in response).toBe(false);
    expect(response.signature).toMatch(/^[A-Za-z0-9_-]{86}$/);
    await verifyInvokeResponseAuth(EMITTER_PUBKEY, response, SESSION_ID);
  });

  it("rejects a tampered success payload (envelope_auth_invalid)", async () => {
    const response = await authenticateInvokeResponse(EMITTER_SEED, responseSuccessInput());
    const tampered = { ...response, payload: { ok: false, result: { id: "n2" } } };
    await rejectsWithKind(
      verifyInvokeResponseAuth(EMITTER_PUBKEY, tampered, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a tampered error envelope (envelope_auth_invalid)", async () => {
    const response = await authenticateInvokeResponse(EMITTER_SEED, responseErrorInput());
    const tampered = {
      ...response,
      error: { code: "internal_error", message: "forged", extensions: {} },
    };
    await rejectsWithKind(
      verifyInvokeResponseAuth(EMITTER_PUBKEY, tampered, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a response carrying BOTH branches (branches are never merged)", async () => {
    const response = await authenticateInvokeResponse(EMITTER_SEED, responseSuccessInput());
    const both = {
      ...response,
      error: { code: "op_unsupported", message: "x", extensions: {} },
    } as unknown as ConnectInvokeResponse;
    await rejectsWithKind(
      verifyInvokeResponseAuth(EMITTER_PUBKEY, both, SESSION_ID),
      "envelope_auth_invalid",
    );
  });

  it("rejects a response carrying NEITHER branch", async () => {
    const response = await authenticateInvokeResponse(EMITTER_SEED, responseSuccessInput());
    const { payload: _payload, ...neither } = response as unknown as Record<string, unknown>;
    await rejectsWithKind(
      verifyInvokeResponseAuth(
        EMITTER_PUBKEY,
        neither as unknown as ConnectInvokeResponse,
        SESSION_ID,
      ),
      "envelope_auth_invalid",
    );
  });

  it("rejects a response missing its signature (envelope_auth_missing)", async () => {
    const response = await authenticateInvokeResponse(EMITTER_SEED, responseSuccessInput());
    const { signature: _signature, ...missing } = response as unknown as Record<string, unknown>;
    await rejectsWithKind(
      verifyInvokeResponseAuth(
        EMITTER_PUBKEY,
        missing as unknown as ConnectInvokeResponse,
        SESSION_ID,
      ),
      "envelope_auth_missing",
    );
  });

  it("rejects a response whose session_id is not bound (envelope_auth_session_unbound)", async () => {
    const response = await authenticateInvokeResponse(EMITTER_SEED, responseErrorInput());
    await rejectsWithKind(
      verifyInvokeResponseAuth(EMITTER_PUBKEY, response, "session-other"),
      "envelope_auth_session_unbound",
    );
  });
});
