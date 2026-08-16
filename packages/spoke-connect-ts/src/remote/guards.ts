/**
 * Wire-shape discrimination guards for the connect envelopes (§Transport
 * framing: envelope types are distinguishable by JSON shape).
 *
 * Isomorphic by design (no `ws` import): these are shared by the Node
 * connect client (`src/node/connect-client.ts`, which re-exports them) and
 * the isomorphic RemoteAdapter + loopback host under `src/remote/`.
 */

import type {
  ConnectHello,
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ConnectSession,
} from "@42ch/spoke-schemas";

/** `ConnectHello` guard: `{protocol_version, peer_id, nonce, host, signature, …}`. */
export function isConnectHello(doc: unknown): doc is ConnectHello {
  return (
    typeof doc === "object" &&
    doc !== null &&
    "protocol_version" in doc &&
    "peer_id" in doc &&
    "nonce" in doc &&
    "host" in doc &&
    "signature" in doc
  );
}

/** `ConnectSession` snapshot guard (wire shape; full field validation happens in the caller). */
export function isConnectSession(doc: unknown): doc is ConnectSession {
  return (
    typeof doc === "object" &&
    doc !== null &&
    "session_id" in doc &&
    "initiator_peer_id" in doc &&
    "responder_peer_id" in doc &&
    "initial_sequence" in doc
  );
}

/** `ConnectInvokeRequest` guard. */
export function isConnectInvokeRequest(doc: unknown): doc is ConnectInvokeRequest {
  return (
    typeof doc === "object" &&
    doc !== null &&
    "session_id" in doc &&
    "sequence" in doc &&
    "request_id" in doc &&
    "op" in doc &&
    "payload" in doc
  );
}

/**
 * `ConnectInvokeResponse` guard (success `payload` branch or error branch).
 *
 * Classification rule (normative `spoke-connect.md` §Request / response
 * classification): an inbound envelope carrying `op` is a
 * `ConnectInvokeRequest` — NEVER a response. A reverse request carries the
 * same correlation echo fields (`session_id` / `sequence` / `request_id`)
 * and a `payload` as the success branch, so without this exclusion it would
 * satisfy the response discriminator and be silently swallowed by a
 * `request_id` demux. No response ever carried `op` per the wire field
 * tables, so rejecting `op`-bearing docs is strictly hardening.
 */
export function isConnectInvokeResponse(doc: unknown): doc is ConnectInvokeResponse {
  return (
    typeof doc === "object" &&
    doc !== null &&
    !("op" in doc) &&
    "session_id" in doc &&
    "sequence" in doc &&
    "request_id" in doc &&
    ("payload" in doc || "error" in doc)
  );
}
