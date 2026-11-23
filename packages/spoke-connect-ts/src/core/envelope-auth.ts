/**
 * Envelope authentication for post-hello connect envelopes over **raw**
 * Ed25519 key bytes (`spoke-connect-session-jcs-v1`,
 * `spoke-connect-invoke-request-jcs-v1`, `spoke-connect-invoke-response-jcs-v1`).
 *
 * Same construction as hello (`spoke-connect-hello-jcs-v1`, see
 * `src/core/hello.ts` / `crates/spoke-connect/src/core/hello_crypto.rs`):
 * the signed object — exactly the keys of the locked field tables
 * (`.agents/specs/connect/spoke-connect.md` §Authenticated field sets) — is
 * canonicalized with RFC 8785 JCS → UTF-8 bytes, signed with the
 * sender's peer-identity Ed25519 private key (the same key used for hello),
 * and the raw 64-byte signature is encoded base64url without padding
 * (exactly 86 characters).
 *
 * Signed-object rules (locked, contract §3):
 * - `extensions` and the `signature` field are excluded from every signed
 *   object.
 * - `ConnectInvokeRequest.auth` is included in the signed object **when
 *   present** on the wire and absent when absent (it is trust-affecting and
 *   MUST be bound).
 * - The `ConnectInvokeResponse` signed object mirrors the wire branch
 *   exactly (`payload` success branch XOR `error` branch) — the two
 *   branches are NEVER merged into one object with optional payload/error;
 *   the discriminator is implicit in which keys are present.
 *
 * Verify is fail-closed (contract §7, in order): presence → canonical
 * base64url round-trip (`decode → encode` equality rejects non-canonical
 * final-char slack bits and padded input) → exact-keys signed-object
 * construction (unknown keys = field-set drift) → JCS → Ed25519 verify with
 * the peer's hello public key → session binding. Every rejection throws
 * `EnvelopeAuthError` with `code: "auth_failed"` and a stable machine kind
 * in `details.kind`: `envelope_auth_missing` / `envelope_auth_invalid` /
 * `envelope_auth_session_unbound`.
 *
 * Key misuse on either side (a secret or public key that is not 32 bytes)
 * throws `CoreError("crypto")`, mirroring `signHelloEd25519` /
 * `verifyHelloEd25519` — the key is supplied by the adapter, never the wire,
 * so a wrong-length key is a local programming error, not a wire rejection,
 * and carries no envelope-auth kind.
 *
 * Module-internal (contract §9): imported by RemoteAdapter / connect-client
 * and the core tests; NOT re-exported from the package root.
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
} from "../crypto.js";
import { CoreError } from "./error.js";

/** Algorithm id: `ConnectSession` snapshot (`spoke-connect-session-jcs-v1`). */
export const ALGORITHM_SESSION_JCS_V1 = "spoke-connect-session-jcs-v1";
/** Algorithm id: `ConnectInvokeRequest` (`spoke-connect-invoke-request-jcs-v1`). */
export const ALGORITHM_INVOKE_REQUEST_JCS_V1 = "spoke-connect-invoke-request-jcs-v1";
/** Algorithm id: `ConnectInvokeResponse` (`spoke-connect-invoke-response-jcs-v1`). */
export const ALGORITHM_INVOKE_RESPONSE_JCS_V1 = "spoke-connect-invoke-response-jcs-v1";

/** Stable machine kind of an envelope-auth rejection (contract §8). */
export type EnvelopeAuthErrorKind =
  | "envelope_auth_missing"
  | "envelope_auth_invalid"
  | "envelope_auth_session_unbound";

/**
 * Envelope-authentication rejection (fail-closed verify path).
 *
 * `code` mirrors the wire `ErrorEnvelope.code` (`auth_failed`, contract §8);
 * the stable machine discriminator is `kind` (also surfaced as
 * `details.kind`). The wire mapping (`INTERNAL_ERROR` reject +
 * `ErrorEnvelope.code: "auth_failed"`) is owned by the adapter.
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

/** Wire `extensions` bag shape (structurally identical to the schema `ExtensionMap`). */
export type EnvelopeExtensions = ConnectSession["extensions"];

/** Input to `authenticateSession`: the session snapshot fields covered by the signature. */
export type SessionSignInput = Omit<ConnectSession, "signature" | "extensions">;

/** Input to `authenticateInvokeRequest`: the request fields covered by the signature (`auth` optional). */
export type InvokeRequestSignInput = Omit<ConnectInvokeRequest, "signature" | "extensions">;

/**
 * Input to `authenticateInvokeResponse`: exactly one of the two response branches.
 * The success branch is selected by the presence of `payload`, which is any-JSON
 * (`OpaqueJson`) on the wire and therefore cannot discriminate as an object.
 */
export type InvokeResponseSignInput =
  | Omit<Extract<ConnectInvokeResponse, { payload: unknown }>, "signature" | "extensions">
  | Omit<Extract<ConnectInvokeResponse, { error: object }>, "signature" | "extensions">;

// invariant: wire key sets are runtime whitelists, mirroring the locked field tables

const SESSION_WIRE_KEYS: readonly string[] = [
  "session_id",
  "initiator_peer_id",
  "responder_peer_id",
  "opened_at",
  "negotiated_capabilities",
  "initial_sequence",
  "signature",
  "extensions",
];

/** invariant: signed fields of a `ConnectSession` (exactly these keys, no others). */
const SESSION_SIGNED_KEYS: readonly string[] = [
  "session_id",
  "initiator_peer_id",
  "responder_peer_id",
  "opened_at",
  "negotiated_capabilities",
  "initial_sequence",
];

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

/** note: signed fields of a `ConnectInvokeRequest` (`auth` conditional, checked separately). */
const INVOKE_REQUEST_SIGNED_KEYS: readonly string[] = [
  "session_id",
  "sequence",
  "request_id",
  "op",
  "payload",
];

const INVOKE_RESPONSE_SUCCESS_WIRE_KEYS: readonly string[] = [
  "session_id",
  "sequence",
  "request_id",
  "payload",
  "signature",
  "extensions",
];

const INVOKE_RESPONSE_ERROR_WIRE_KEYS: readonly string[] = [
  "session_id",
  "sequence",
  "request_id",
  "error",
  "signature",
  "extensions",
];

/** note: signed fields common to both `ConnectInvokeResponse` branches (branch key checked separately). */
const INVOKE_RESPONSE_SIGNED_KEYS: readonly string[] = [
  "session_id",
  "sequence",
  "request_id",
];

// invariant: signed-object construction uses exact keys, no others

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
 * note: fail-closed wire-shape guard: every wire key must be in the allowed set
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

/**
 * note: one root-relative wire path of a signed object (object keys; array
 * elements are transparent). Mirrors `RevisionPath` in
 * `crates/spoke-connect/src/core/jcs.rs`.
 */
type RevisionPath = readonly string[];

/** note: inclusive revision endpoint (`2^53`) — the Rust `EXACT_INTEGER_BOUND`. */
const EXACT_INTEGER_BOUND = 2 ** 53;

/** note: the path state of a subtree no declared revision path reaches. */
const NO_REVISION_PATHS: readonly RevisionPath[] = [];

/**
 * note: request paths that own a revision integer, per op — the Rust
 * `REQUEST_REVISION_PATHS` tables (`.agents/specs/connect/spoke-connect.md`
 * §Signature canonicalization). A member outside these paths is product
 * data: its key spelling never assigns a revision domain.
 */
const REQUEST_REVISION_PATHS: Readonly<Record<string, readonly RevisionPath[]>> = {
  "history.revisions.get": [["payload", "revision"]],
  "history.revisions.prune": [["payload", "keep"]],
  "knowledge.adopt": [["payload", "source_revision"]],
  "memory.candidates.dismiss": [["payload", "expected_revision"]],
  "memory.candidates.review": [["payload", "expected_revision"]],
  "memory.grants.revoke": [["payload", "expected_revision"]],
  "memory.imports.stage": [
    ["payload", "chunk", "candidates", "revision"],
    ["payload", "chunk", "grants", "revision"],
    ["payload", "chunk", "profiles", "revision"],
    ["payload", "chunk", "records", "import_ref", "source_revision"],
    ["payload", "chunk", "records", "revision"],
    ["payload", "chunk", "reflections", "revision"],
    ["payload", "chunk", "subjects", "revision"],
  ],
  "memory.profile.update": [["payload", "expected_revision"]],
  "memory.records.delete": [["payload", "expected_revision"]],
  "memory.records.promote": [["payload", "expected_revision"]],
  "memory.records.update": [["payload", "expected_revision"]],
  "memory.spaces.freeze": [["payload", "expected_revision"]],
  "memory.spaces.unfreeze": [["payload", "expected_revision"]],
  "memory.subjects.fence": [["payload", "expected_revision"]],
  "port.knowledge.put": [
    ["payload", "entry", "revision"],
    ["payload", "entry", "adoption_anchor", "source_revision"],
    ["payload", "expected_base_revision"],
  ],
  "port.relation.put": [
    ["payload", "relation", "revision"],
    ["payload", "expected_base_revision"],
  ],
  promote: [
    ["payload", "candidate", "revision"],
    ["payload", "candidate", "adoption_anchor", "source_revision"],
  ],
  relate: [["payload", "relation", "revision"]],
  upsert: [
    ["payload", "knowledge_entries", "revision"],
    ["payload", "knowledge_entries", "adoption_anchor", "source_revision"],
  ],
};

/**
 * note: response paths that own a revision integer — the direction-wide
 * union of the Rust `RESPONSE_REVISION_PATHS` table (a response envelope
 * carries no op, so the union over the response schemas is the exact set),
 * including the adoption refusal at both owning branches.
 */
const RESPONSE_REVISION_PATHS: readonly RevisionPath[] = [
  ["error", "details", "adopted_revision"],
  ["payload", "adoption_anchor", "source_revision"],
  ["payload", "candidate", "revision"],
  ["payload", "candidates", "adoption_anchor", "source_revision"],
  ["payload", "candidates", "revision"],
  ["payload", "chunk", "candidates", "revision"],
  ["payload", "chunk", "grants", "revision"],
  ["payload", "chunk", "profiles", "revision"],
  ["payload", "chunk", "records", "import_ref", "source_revision"],
  ["payload", "chunk", "records", "revision"],
  ["payload", "chunk", "reflections", "revision"],
  ["payload", "chunk", "subjects", "revision"],
  ["payload", "deleted"],
  ["payload", "entry", "adoption_anchor", "source_revision"],
  ["payload", "entry", "revision"],
  ["payload", "error", "details", "adopted_revision"],
  ["payload", "floor_revision"],
  ["payload", "grants", "revision"],
  ["payload", "knowledge_entries", "adoption_anchor", "source_revision"],
  ["payload", "knowledge_entries", "revision"],
  ["payload", "knowledge_entry", "adoption_anchor", "source_revision"],
  ["payload", "knowledge_entry", "revision"],
  ["payload", "profiles", "revision"],
  ["payload", "receipt", "result", "revision"],
  ["payload", "record", "import_ref", "source_revision"],
  ["payload", "record", "revision"],
  ["payload", "records", "import_ref", "source_revision"],
  ["payload", "records", "revision"],
  ["payload", "reflections", "revision"],
  ["payload", "relation", "revision"],
  ["payload", "result", "revision"],
  ["payload", "revision"],
  ["payload", "snapshot", "revision"],
  ["payload", "snapshot", "snapshot", "adoption_anchor", "source_revision"],
  ["payload", "snapshot", "snapshot", "revision"],
  ["payload", "spaces", "revision"],
  ["payload", "subject", "revision"],
];

/** note: the owning revision paths of one signed object (Rust `revision_paths`). */
function revisionPaths(
  signedObject: Record<string, unknown>,
): readonly RevisionPath[] {
  if (typeof signedObject.op === "string") {
    return REQUEST_REVISION_PATHS[signedObject.op] ?? [];
  }
  if ("payload" in signedObject || "error" in signedObject) {
    return RESPONSE_REVISION_PATHS;
  }
  return [];
}

/**
 * why: the signed-boundary member domain (Rust `core/jcs.rs` parity, qc2
 * C-QC2-01): an integer past `2^53 - 1` is signable only at a declared
 * revision path, and there only at the exact `±2^53` endpoint — every other
 * value has no shared IEEE-754 identity with the peer, so signing refuses it
 * instead of rounding it into the signed identity.
 *
 * The refusal mirrors the Rust integer-token range: `serde_json` keeps an
 * integer token as `u64`/`i64` until that range overflows and only then falls
 * back to `f64`, so `[2^53, 2^64)` and `[-2^63, -2^53]` are the values
 * `serde_jcs` refuses; a wider `f64` keeps the shared spelling both stacks
 * emit for it.
 */
function assertRevisionDomain(
  value: unknown,
  paths: readonly RevisionPath[],
  depth = 0,
  revision = false,
): void {
  if (
    typeof value === "number" &&
    (value >= EXACT_INTEGER_BOUND
      ? value < 2 ** 64
      : value <= -EXACT_INTEGER_BOUND && value >= -(2 ** 63))
  ) {
    if (!revision || Math.abs(value) !== EXACT_INTEGER_BOUND) {
      throw new EnvelopeAuthError(
        "envelope_auth_invalid",
        `integer ${String(value)} is outside the revision member domain of the signed object`,
      );
    }
    return;
  }
  if (Array.isArray(value)) {
    // note: an array element shares its array's path — positional indexes are
    // not wire members, so the element never owns a revision slot itself.
    for (const element of value) {
      assertRevisionDomain(element, paths, depth, false);
    }
    return;
  }
  if (typeof value !== "object" || value === null) {
    return;
  }
  for (const [key, member] of Object.entries(value)) {
    let terminal = false;
    let continues = false;
    for (const path of paths) {
      if (path[depth] !== key) {
        continue;
      }
      if (path.length === depth + 1) {
        terminal = true;
      } else {
        continues = true;
      }
    }
    // note: a declared path owns this key; every other member is product
    // data whose integers still face the refusal, so the walk descends with
    // no path state instead of skipping the subtree.
    if (terminal || continues) {
      assertRevisionDomain(member, paths, depth + 1, terminal);
    } else {
      assertRevisionDomain(member, NO_REVISION_PATHS, 0, false);
    }
  }
}

function jcsBytes(signedObject: Record<string, unknown>): Uint8Array {
  assertRevisionDomain(signedObject, revisionPaths(signedObject));
  const jcs = canonicalize(signedObject);
  if (jcs === undefined) {
    throw new EnvelopeAuthError(
      "envelope_auth_invalid",
      "signed object is not JSON-serializable",
    );
  }
  return new TextEncoder().encode(jcs);
}

async function signEnvelope(
  secret: Uint8Array,
  signedObject: Record<string, unknown>,
): Promise<string> {
  if (secret.length !== 32) {
    throw new CoreError("crypto", "secret must be 32 bytes");
  }
  const bytes = jcsBytes(signedObject);
  return base64UrlEncode(await signEd25519(secret, bytes));
}

/**
 * why: verify steps 1–2 (locked order): the `signature` field must be present
 * and non-empty (`envelope_auth_missing`), then it must decode and
 * round-trip through `encode(decode(sig)) === sig` — the unique canonical
 * RFC 4648 base64url no-padding encoding, rejecting alternate encodings of
 * the final character's slack bits and padded input — and be exactly 64
 * bytes (`envelope_auth_invalid`).
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
 * note: verify steps 4–5 (locked order): JCS-canonicalize the signed object and
 * Ed25519-verify the decoded signature against the peer's hello public key.
 * A non-32-byte public key is adapter-supplied misuse → `CoreError("crypto")`
 * (mirrors `verifyHelloEd25519`), not a wire rejection.
 */
async function verifyCanonicalSignature(
  publicKey: Uint8Array,
  signedObject: Record<string, unknown>,
  signature: Uint8Array,
): Promise<void> {
  if (publicKey.length !== 32) {
    throw new CoreError("crypto", "public key must be 32 bytes");
  }
  const bytes = jcsBytes(signedObject);
  if (!(await verifyEd25519(publicKey, bytes, signature))) {
    throw new EnvelopeAuthError(
      "envelope_auth_invalid",
      "signature does not verify",
    );
  }
}

/**
 * Sign a `ConnectSession` snapshot with a raw 32-byte Ed25519 secret key,
 * producing the full wire envelope (`spoke-connect-session-jcs-v1`). The
 * signed object covers exactly `{session_id, initiator_peer_id,
 * responder_peer_id, opened_at, negotiated_capabilities, initial_sequence}`.
 */
export async function authenticateSession(
  secret: Uint8Array,
  session: SessionSignInput,
  extensions: EnvelopeExtensions = {},
): Promise<ConnectSession> {
  const signature = await signEnvelope(secret, sessionSignedObject(session));
  return {
    ...session,
    signature,
    extensions,
  };
}

/**
 * Verify a received `ConnectSession` against a raw Ed25519 public key (the
 * emitter's hello key). Fail-closed per contract §7; on success additionally
 * asserts the snapshot's `initiator_peer_id` / `responder_peer_id` match the
 * authenticated hellos of the current session (step 6 — mismatch is
 * `envelope_auth_session_unbound`, after the signature has verified).
 */
export async function verifySessionAuth(
  publicKey: Uint8Array,
  session: ConnectSession,
  expectedInitiatorPeerId: string,
  expectedResponderPeerId: string,
): Promise<void> {
  const wire = session as unknown as Record<string, unknown>;
  const signature = requireSignature(wire.signature);
  assertWireKeys(wire, SESSION_WIRE_KEYS, SESSION_SIGNED_KEYS);
  await verifyCanonicalSignature(publicKey, sessionSignedObject(session), signature);
  if (
    session.initiator_peer_id !== expectedInitiatorPeerId ||
    session.responder_peer_id !== expectedResponderPeerId
  ) {
    throw new EnvelopeAuthError(
      "envelope_auth_session_unbound",
      `session peer ids (${session.initiator_peer_id}, ${session.responder_peer_id}) do not match the authenticated hellos (${expectedInitiatorPeerId}, ${expectedResponderPeerId})`,
    );
  }
}

/**
 * Sign a `ConnectInvokeRequest` with a raw 32-byte Ed25519 secret key,
 * producing the full wire envelope (`spoke-connect-invoke-request-jcs-v1`).
 * The signed object covers `{session_id, sequence, request_id, op, payload}`
 * plus `auth` **only when present** on the input (conditional inclusion —
 * `auth` is trust-affecting and MUST be bound).
 */
export async function authenticateInvokeRequest(
  secret: Uint8Array,
  request: InvokeRequestSignInput,
  extensions: EnvelopeExtensions = {},
): Promise<ConnectInvokeRequest> {
  const signature = await signEnvelope(secret, invokeRequestSignedObject(request));
  return {
    ...request,
    signature,
    extensions,
  };
}

/**
 * Verify a received `ConnectInvokeRequest` against a raw Ed25519 public key
 * (the session peer's hello key). Fail-closed per contract §7; on success
 * additionally asserts the envelope's `session_id` equals the session bound
 * at establish (step 6 — `envelope_auth_session_unbound` on mismatch; the
 * adapter resolves the peer key from the bound session, so the signer-is-
 * session-peer check is the key itself).
 */
export async function verifyInvokeRequestAuth(
  publicKey: Uint8Array,
  request: ConnectInvokeRequest,
  expectedSessionId: string,
): Promise<void> {
  const wire = request as unknown as Record<string, unknown>;
  const signature = requireSignature(wire.signature);
  assertWireKeys(wire, INVOKE_REQUEST_WIRE_KEYS, INVOKE_REQUEST_SIGNED_KEYS);
  await verifyCanonicalSignature(publicKey, invokeRequestSignedObject(request), signature);
  if (request.session_id !== expectedSessionId) {
    throw new EnvelopeAuthError(
      "envelope_auth_session_unbound",
      `session_id ${request.session_id} is not bound to session ${expectedSessionId}`,
    );
  }
}

/**
 * Sign a `ConnectInvokeResponse` with a raw 32-byte Ed25519 secret key,
 * producing the full wire envelope (`spoke-connect-invoke-response-jcs-v1`).
 * The signed object mirrors the wire branch exactly — `{session_id,
 * sequence, request_id, payload}` for the success branch, `{session_id,
 * sequence, request_id, error}` for the error branch. The branches are never
 * merged.
 */
export async function authenticateInvokeResponse(
  secret: Uint8Array,
  response: InvokeResponseSignInput,
  extensions: EnvelopeExtensions = {},
): Promise<ConnectInvokeResponse> {
  if ("payload" in response) {
    const signature = await signEnvelope(secret, {
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
  const signature = await signEnvelope(secret, {
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
}

/**
 * Verify a received `ConnectInvokeResponse` against a raw Ed25519 public key
 * (the session peer's hello key). Fail-closed per contract §7: the response
 * must carry exactly one of `payload` / `error` (never both — the signed
 * object mirrors the wire branch, never a merged object), the signed object
 * must have exact keys for that branch, and on success the envelope's
 * `session_id` must equal the bound session (step 6).
 */
export async function verifyInvokeResponseAuth(
  publicKey: Uint8Array,
  response: ConnectInvokeResponse,
  expectedSessionId: string,
): Promise<void> {
  const wire = response as unknown as Record<string, unknown>;
  const signature = requireSignature(wire.signature);

  const hasPayload = "payload" in wire;
  const hasError = "error" in wire;
  if (hasPayload === hasError) {
    throw new EnvelopeAuthError(
      "envelope_auth_invalid",
      "invoke response must carry exactly one of payload or error",
    );
  }

  let signedObject: Record<string, unknown>;
  if (hasPayload) {
    assertWireKeys(wire, INVOKE_RESPONSE_SUCCESS_WIRE_KEYS, [
      ...INVOKE_RESPONSE_SIGNED_KEYS,
      "payload",
    ]);
    signedObject = {
      session_id: wire.session_id,
      sequence: wire.sequence,
      request_id: wire.request_id,
      payload: wire.payload,
    };
  } else {
    assertWireKeys(wire, INVOKE_RESPONSE_ERROR_WIRE_KEYS, [
      ...INVOKE_RESPONSE_SIGNED_KEYS,
      "error",
    ]);
    signedObject = {
      session_id: wire.session_id,
      sequence: wire.sequence,
      request_id: wire.request_id,
      error: wire.error,
    };
  }

  await verifyCanonicalSignature(publicKey, signedObject, signature);

  if (wire.session_id !== expectedSessionId) {
    throw new EnvelopeAuthError(
      "envelope_auth_session_unbound",
      `session_id ${wire.session_id} is not bound to session ${expectedSessionId}`,
    );
  }
}
