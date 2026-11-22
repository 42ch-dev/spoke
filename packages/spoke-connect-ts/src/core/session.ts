/**
 * Thin session helper (per-session core state, AD-P0-3).
 *
 * Holds the per-direction sequence counters, `session_id`,
 * `negotiated_capabilities`, and the two session peer ids bound to the
 * authenticated hellos (see `.agents/specs/connect/spoke-connect.md`
 * contract §Session-core state machine: Established ⇒
 * outbound counter = 0, inbound expected = 0). This is the optional thin
 * helper, not a full `Disconnected→…→Closed` state-machine class — the
 * guards it wraps (sequence, dispatch gate) are the pure core port; the
 * transport layer owns dialing and stream lifecycle.
 *
 * `negotiatedCapabilities` implements the spec's "agreed subset": the
 * intersection of both hosts' manifest `capabilities[]` (which naturally
 * includes `spoke-connect` only when both declare it).
 */

import { dispatchAllowed as coreDispatchAllowed } from "./dispatch.js";
import { InboundSequence, OutboundSequence } from "./sequence.js";

/** Intersection (agreed subset) of two hosts' capability lists, in local order. */
export function negotiatedCapabilities(
  local: readonly string[],
  remote: readonly string[],
): string[] {
  const remoteSet = new Set(remote);
  return local.filter((capability) => remoteSet.has(capability));
}

export interface SessionOptions {
  session_id: string;
  initiator_peer_id: string;
  responder_peer_id: string;
  /** note: agreed subset of both hosts' capabilities (spec §Session-core state machine). */
  negotiated_capabilities: readonly string[];
}

/** Per-session core state: counters, session id, negotiated capabilities, peer ids. */
export class Session {
  readonly session_id: string;
  readonly initiator_peer_id: string;
  readonly responder_peer_id: string;
  readonly negotiated_capabilities: readonly string[];

  private readonly outbound: OutboundSequence;
  private readonly inbound: InboundSequence;

  constructor(options: SessionOptions) {
    this.session_id = options.session_id;
    this.initiator_peer_id = options.initiator_peer_id;
    this.responder_peer_id = options.responder_peer_id;
    this.negotiated_capabilities = options.negotiated_capabilities;
    this.outbound = new OutboundSequence();
    this.inbound = new InboundSequence();
  }

  get nextOutboundSequence(): number {
    return this.outbound.next();
  }

  get nextExpectedInboundSequence(): number {
    return this.inbound.nextExpected();
  }

  /**
   * note: atomically-in-spirit assign the next outbound sequence (first = 0).
   * Throws `sequence_exhausted` past 2⁵³−1 — sequences never wrap; the
   * caller must close the session.
   */
  allocateOutboundSequence(): number {
    return this.outbound.allocate();
  }

  /**
   * note: accept an inbound invoke sequence iff it is the next expected one;
   * throws `inbound_sequence_mismatch` otherwise and leaves the expectation
   * unchanged (the caller must reject the invoke without dispatching it).
   */
  acceptInboundSequence(sequence: number): void {
    this.inbound.advance(sequence);
  }

  /**
   * Validate an inbound invoke sequence against the next expected one
   * WITHOUT advancing the expectation (auth-before-advance — envelope-auth
   * contract §7 amendment: the wire position is checked at acceptance, but
   * the counter only advances after envelope-auth verify passes). Throws
   * `inbound_sequence_mismatch` on mismatch and leaves the expectation
   * unchanged, exactly like `acceptInboundSequence`.
   */
  peekInboundSequence(sequence: number): void {
    this.inbound.peek(sequence);
  }

  dispatchAllowed(op: string): boolean {
    return coreDispatchAllowed(op, this.negotiated_capabilities);
  }
}
