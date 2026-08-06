/**
 * Spec-derived envelope authentication (protocol_version 2) for the demo
 * responder — implemented over PUBLIC `@42ch/spoke-connect` primitives
 * (the library keeps its own helpers module-internal by design,
 * `spoke-remote-adapter.md` D10).
 *
 * Normative construction (`.mstar/specs/spoke-connect.md` §Envelope
 * authentication (protocol_version 2)) is identical to
 * `spoke-connect-hello-jcs-v1`: RFC 8785 JCS → UTF-8 bytes → Ed25519
 * sign/verify → base64url without padding. The three algorithm ids and the
 * authenticated field sets are locked:
 *
 * - `spoke-connect-session-jcs-v1` — `ConnectSession`:
 *   `{session_id, initiator_peer_id, responder_peer_id, opened_at,
 *   negotiated_capabilities, initial_sequence}`;
 * - `spoke-connect-invoke-request-jcs-v1` — `ConnectInvokeRequest`:
 *   `{session_id, sequence, request_id, op, payload}` and additionally
 *   `auth` WHEN present on the wire (trust-affecting; MUST be bound);
 * - `spoke-connect-invoke-response-jcs-v1` — `ConnectInvokeResponse`:
 *   success branch `{session_id, sequence, request_id, payload}` XOR error
 *   branch `{session_id, sequence, request_id, error}` — the two branches
 *   are signed independently, never merged.
 *
 * `extensions` and `signature` are excluded from every signed object.
 * Verify is fail-closed (spec §Verify rules, in order): presence → canonical
 * base64url round-trip (decode → encode equality) → exact-keys signed-object
 * construction → JCS → Ed25519 verify with the peer's hello public key →
 * session binding. Every rejection throws `EnvelopeAuthError` with
 * `code: "auth_failed"` and a stable machine kind in `details.kind`
 * (`envelope_auth_missing` / `envelope_auth_invalid` /
 * `envelope_auth_session_unbound`). Key misuse (wrong-length key) is a
 * local programming error and fails loudly — never a wire rejection.
 */

import type {
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ConnectSession,
} from "@42ch/spoke-schemas";

import canonicalize from "canonicalize";

import {
  base64UrlDecode,
  base64UrlEncode,
  signEd25519,
  verifyEd25519,
} from "@42ch/spoke-connect";

/** Algorithm id: `ConnectSession` snapshot (`spoke-connect-session-jcs-v1`). */
export const ALGORITHM_SESSION_JCS_V1 = "spoke-connect-session-jcs-v1";
/** Algorithm id: `ConnectInvokeRequest` (`spoke-connect-invoke-request-jcs-v1`). */
export const ALGORITHM_INVOKE_REQUEST_JCS_V1 =
  "spoke-connect-invoke-request-jcs-v1";
/** Algorithm id: `ConnectInvokeResponse` (`spoke-connect-invoke-response-jcs-v1`). */
export const ALGORITHM_INVOKE_RESPONSE_JCS_V1 =
  "spoke-connect-invoke-response-jcs-v1";

/** Stable machine kind of an envelope-auth rejection (spec §Error mapping). */
export type EnvelopeAuthErrorKind =
  | "envelope_auth_missing"
  | "envelope_auth_invalid"
  | "envelope_auth_session_unbound";

/**
 * Envelope-authentication rejection (fail-closed verify path). `code`
 * mirrors the wire `ErrorEnvelope.code` (`auth_failed`); the stable machine
 * discriminator is `kind` (surfaced as `details.kind`).
 */
export class EnvelopeAuthError extends Error {
  readonly code = "auth_failed" as const;
  readonly kind: EnvelopeAuthErrorKind;
  readonly details: { kind: EnvelopeAuthErrorKind };

  constructor(kind: EnvelopeAuthErrorKind, message?: string) {
    super(message ?? `envelope authentication failed (${kind})`);
    this.name = "EnvelopeAuthError";
    this.kind = kind;
    this.details = { kind };
  }
}

/** Wire `extensions` bag shape (structurally the schema `ExtensionMap`). */
export type EnvelopeExtensions = ConnectSession["extensions"];

/** The session snapshot fields covered by the signature. */
export type SessionSignInput = Omit<ConnectSession, "signature" | "extensions">;

/** The request fields covered by the signature (`auth` optional). */
export type InvokeRequestSignInput = Omit<
  ConnectInvokeRequest,
  "signature" | "extensions"
>;

/** The response fields covered by the signature: exactly one wire branch. */
export type InvokeResponseSignInput =
  | Omit<
      Extract<ConnectInvokeResponse, { payload: object }>,
      "signature" | "extensions"
    >
  | Omit<
      Extract<ConnectInvokeResponse, { error: object }>,
      "signature" | "extensions"
    >;

// ── wire key sets (runtime whitelists, mirror the locked field tables) ─────

/** Allowed wire keys of a `ConnectInvokeRequest` envelope (`auth` optional). */
const INVOKE_REQUEST_WIRE_KEYS: readonly string[] = [
  "session_id",
  "sequence",
  "request_id",
  "op",
  "payload",
  "auth",
  "signature",
  "extensions",
];

/** Signed fields of a `ConnectInvokeRequest` (`auth` conditional, checked separately). */
const INVOKE_REQUEST_SIGNED_KEYS: readonly string[] = [
  "session_id",
  "sequence",
  "request_id",
  "op",
  "payload",
];

// ── signed-object construction (exact keys, no others) ─────────────────────

/** The session signed object: exactly the six locked session fields. */
function sessionSignedObject(session: SessionSignInput): Record<string, unknown> {
  return {
    session_id: session.session_id,
    initiator_peer_id: session.initiator_peer_id,
    responder_peer_id: session.responder_peer_id,
    opened_at: session.opened_at,
    negotiated_capabilities: session.negotiated_capabilities,
    initial_sequence: session.initial_sequence,
  };
}

/** The invoke-request signed object: five locked fields + `auth` when present. */
function invokeRequestSignedObject(
  request: InvokeRequestSignInput,
): Record<string, unknown> {
  const signedObject: Record<string, unknown> = {
    session_id: request.session_id,
    sequence: request.sequence,
    request_id: request.request_id,
    op: request.op,
    payload: request.payload,
  };
  if (request.auth !== undefined) {
    signedObject.auth = request.auth;
  }
  return signedObject;
}

/**
 * Fail-closed wire-shape guard: every wire key must be in the allowed set
 * (unknown keys = field-set drift ⇒ `envelope_auth_invalid`) and every
 * signed field must be present (a missing signed field is a malformed
 * signed object ⇒ `envelope_auth_invalid`).
 */
function assertWireKeys(
  wire: Record<string, unknown>,
  allowed: readonly string[],
  required: readonly string[],
): void {
  for (const key of Object.keys(wire)) {
    if (!allowed.includes(key)) {
      throw new EnvelopeAuthError(
        "envelope_auth_invalid",
        `unknown key ${key} in signed envelope (field-set drift)`,
      );
    }
  }
  for (const key of required) {
    if (wire[key] === undefined) {
      throw new EnvelopeAuthError(
        "envelope_auth_invalid",
        `missing signed field ${key}`,
      );
    }
  }
}

// ── JCS + signature primitives ─────────────────────────────────────────────

/** RFC 8785 JCS-canonicalize the signed object to UTF-8 bytes. */
function jcsBytes(signedObject: Record<string, unknown>): Uint8Array {
  const jcs = canonicalize(signedObject);
  if (jcs === undefined) {
    throw new EnvelopeAuthError(
      "envelope_auth_invalid",
      "signed object is not JSON-serializable",
    );
  }
  return new TextEncoder().encode(jcs);
}

/** Sign the JCS bytes with a raw 32-byte Ed25519 seed → canonical base64url. */
async function signEnvelope(
  secret: Uint8Array,
  signedObject: Record<string, unknown>,
): Promise<string> {
  if (secret.length !== 32) {
    throw new Error("secret must be 32 bytes");
  }
  return base64UrlEncode(await signEd25519(secret, jcsBytes(signedObject)));
}

/**
 * Verify steps 1–2 (locked order, spec §Verify rules): the `signature`
 * field must be present and non-empty (`envelope_auth_missing`), then it
 * must decode and round-trip through `encode(decode(sig)) === sig` — the
 * unique canonical RFC 4648 base64url no-padding encoding, rejecting
 * alternate encodings of the final character's slack bits and padded input
 * — and be exactly 64 bytes (`envelope_auth_invalid`).
 */
function requireSignature(signatureField: unknown): Uint8Array {
  if (typeof signatureField !== "string" || signatureField === "") {
    throw new EnvelopeAuthError(
      "envelope_auth_missing",
      "envelope is missing a signature",
    );
  }
  let signature: Uint8Array;
  try {
    signature = base64UrlDecode(signatureField);
  } catch {
    throw new EnvelopeAuthError(
      "envelope_auth_invalid",
      "signature is not valid base64url",
    );
  }
  if (base64UrlEncode(signature) !== signatureField) {
    throw new EnvelopeAuthError(
      "envelope_auth_invalid",
      "signature is not canonical base64url (no padding)",
    );
  }
  if (signature.length !== 64) {
    throw new EnvelopeAuthError(
      "envelope_auth_invalid",
      "signature is not 64 bytes (86-char base64url expected)",
    );
  }
  return signature;
}

/**
 * Verify steps 4–5 (locked order): JCS-canonicalize the signed object and
 * Ed25519-verify the decoded signature against the peer's hello public key.
 * A non-32-byte public key is host-supplied misuse — fail loudly, mirroring
 * `verifyHelloEd25519` — not a wire rejection.
 */
async function verifyCanonicalSignature(
  publicKey: Uint8Array,
  signedObject: Record<string, unknown>,
  signature: Uint8Array,
): Promise<void> {
  if (publicKey.length !== 32) {
    throw new Error("public key must be 32 bytes");
  }
  if (!(await verifyEd25519(publicKey, jcsBytes(signedObject), signature))) {
    throw new EnvelopeAuthError(
      "envelope_auth_invalid",
      "signature does not verify",
    );
  }
}

// ── the responder's envelope-auth surface ──────────────────────────────────

/**
 * The responder-side envelope-auth helpers bound to one Ed25519 identity
 * (the host seed). The responder SIGNS the session snapshot it emits and
 * every invoke response, and VERIFIES every inbound invoke request; it does
 * NOT verify the session snapshot it emits — that is the client's job — so
 * there is no `verifySessionAuth` here (spec §Enforcement).
 */
export interface EnvelopeAuth {
  /**
   * Sign a `ConnectSession` snapshot, producing the full wire envelope
   * (`spoke-connect-session-jcs-v1`) over the locked 6-field set.
   */
  authenticateSession(
    session: SessionSignInput,
    extensions?: EnvelopeExtensions,
  ): Promise<ConnectSession>;
  /**
   * Sign a `ConnectInvokeResponse`, producing the full wire envelope
   * (`spoke-connect-invoke-response-jcs-v1`). The signed object mirrors the
   * wire branch exactly (payload XOR error) — the branches are never merged.
   */
  authenticateInvokeResponse(
    response: InvokeResponseSignInput,
    extensions?: EnvelopeExtensions,
  ): Promise<ConnectInvokeResponse>;
  /**
   * Verify a received `ConnectInvokeRequest` against the peer's hello
   * Ed25519 public key (`spoke-connect-invoke-request-jcs-v1`). Fail-closed
   * per the spec §Verify rules; on success additionally asserts the
   * envelope's `session_id` equals the session bound at establish (step 6 —
   * `envelope_auth_session_unbound` on mismatch).
   */
  verifyInvokeRequestAuth(
    publicKey: Uint8Array,
    request: ConnectInvokeRequest,
    expectedSessionId: string,
  ): Promise<void>;
}

/** Bind the envelope-auth helpers to one responder Ed25519 seed. */
export function createEnvelopeAuth(options: { seed: Uint8Array }): EnvelopeAuth {
  const { seed } = options;

  return {
    async authenticateSession(
      session: SessionSignInput,
      extensions: EnvelopeExtensions = {},
    ): Promise<ConnectSession> {
      const signature = await signEnvelope(seed, sessionSignedObject(session));
      return { ...session, signature, extensions };
    },

    async authenticateInvokeResponse(
      response: InvokeResponseSignInput,
      extensions: EnvelopeExtensions = {},
    ): Promise<ConnectInvokeResponse> {
      if ("payload" in response) {
        const signature = await signEnvelope(seed, {
          session_id: response.session_id,
          sequence: response.sequence,
          request_id: response.request_id,
          payload: response.payload,
        });
        return {
          session_id: response.session_id,
          sequence: response.sequence,
          request_id: response.request_id,
          payload: response.payload,
          signature,
          extensions,
        };
      }
      const signature = await signEnvelope(seed, {
        session_id: response.session_id,
        sequence: response.sequence,
        request_id: response.request_id,
        error: response.error,
      });
      return {
        session_id: response.session_id,
        sequence: response.sequence,
        request_id: response.request_id,
        error: response.error,
        signature,
        extensions,
      };
    },

    async verifyInvokeRequestAuth(
      publicKey: Uint8Array,
      request: ConnectInvokeRequest,
      expectedSessionId: string,
    ): Promise<void> {
      const wire = request as unknown as Record<string, unknown>;
      const signature = requireSignature(wire.signature);
      assertWireKeys(wire, INVOKE_REQUEST_WIRE_KEYS, INVOKE_REQUEST_SIGNED_KEYS);
      await verifyCanonicalSignature(
        publicKey,
        invokeRequestSignedObject(request),
        signature,
      );
      if (request.session_id !== expectedSessionId) {
        throw new EnvelopeAuthError(
          "envelope_auth_session_unbound",
          `session_id ${request.session_id} is not bound to session ${expectedSessionId}`,
        );
      }
    },
  };
}
