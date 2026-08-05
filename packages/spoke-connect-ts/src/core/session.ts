/**
 * Thin session helper (per-session core state, AD-P0-3).
 *
 * Holds the per-direction sequence counters, `session_id`,
 * `negotiated_capabilities`, and the two session peer ids bound to the
 * authenticated hellos (spec §Session-core state machine: Established ⇒
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
  /** A-assigned or agreed session id (wire snapshot `session_id`). */
  session_id: string;
  /** Peer that dialed / sent first hello. */
  initiator_peer_id: string;
  /** Peer that accepted. */
  responder_peer_id: string;
  /** Agreed subset of both hosts' capabilities (spec §Session-core state machine). */
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
    // Established state: outbound counter = 0, inbound expected = 0.
    this.outbound = new OutboundSequence();
    this.inbound = new InboundSequence();
  }

  /** The next outbound sequence that `allocateOutboundSequence` will assign. */
  get nextOutboundSequence(): number {
    return this.outbound.next();
  }

  /** The next inbound sequence `acceptInboundSequence` will accept. */
  get nextExpectedInboundSequence(): number {
    return this.inbound.nextExpected();
  }

  /**
   * Atomically-in-spirit assign the next outbound sequence (first = 0).
   * Throws `sequence_exhausted` past 2⁵³−1 — sequences never wrap; the
   * caller must close the session.
   */
  allocateOutboundSequence(): number {
    return this.outbound.allocate();
  }

  /**
   * Accept an inbound invoke sequence iff it is the next expected one;
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

  /** Dispatch gate: does this session's negotiated capabilities authorize `op`? */
  dispatchAllowed(op: string): boolean {
    return coreDispatchAllowed(op, this.negotiated_capabilities);
  }
}
