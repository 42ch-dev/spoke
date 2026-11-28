/**
 * Per-direction session sequence counters (pure; no atomics).
 *
 * Ported from `crates/spoke-connect/src/core/sequence.rs`; see
 * `.agents/specs/connect/spoke-connect.md` contract §Ordering semantics / §Session-core state
 * machine: each peer maintains its **own** outbound counter per session
 * starting at 0; inbound invokes are accepted iff
 * `sequence == next_expected_inbound` (start 0) and then advance by 1. When
 * the next outbound sequence would exceed 2⁵³−1 (the JSON-safe wire
 * maximum), the session MUST be closed and reopened — **no wrap-around**.
 *
 * Pure single-threaded counters; a transport needing concurrent allocation
 * wraps them in its own synchronization.
 */

import { CoreInvokeError } from "./error.js";

/** Maximum outbound invoke sequence per session: 2⁵³−1, the JSON-safe wire maximum. */
export const MAX_SEQUENCE: number = 2 ** 53 - 1;

/** Outbound sequence counter, starting at 0. */
export class OutboundSequence {
  private nextValue: number;

  constructor() {
    this.nextValue = 0;
  }

  allocate(): number {
    const sequence = this.nextValue;
    if (sequence > MAX_SEQUENCE) {
      throw new CoreInvokeError("sequence_exhausted");
    }
    this.nextValue = sequence + 1;
    return sequence;
  }

  next(): number {
    return this.nextValue;
  }

  /**
   * @internal test-only — mirrors Rust `#[cfg(test)] set_next`. Not part
   * of the public session-core API; production transports allocate
   * sequences only through `allocate()` / `Session.allocateOutboundSequence()`.
   */
  setNext(value: number): void {
    this.nextValue = value;
  }
}

/** Inbound sequence expectation (receiver side), starting at 0. */
export class InboundSequence {
  private nextExpectedValue: number;

  constructor() {
    this.nextExpectedValue = 0;
  }

  advance(sequence: number): number {
    if (sequence < 0) {
      throw new CoreInvokeError(
        "inbound_sequence_mismatch",
        `inbound sequence ${sequence} is not the next expected ${this.nextExpectedValue}`,
        { expected: this.nextExpectedValue, actual: sequence },
      );
    }
    if (sequence !== this.nextExpectedValue) {
      throw new CoreInvokeError(
        "inbound_sequence_mismatch",
        `inbound sequence ${sequence} is not the next expected ${this.nextExpectedValue}`,
        { expected: this.nextExpectedValue, actual: sequence },
      );
    }
    this.nextExpectedValue += 1;
    return this.nextExpectedValue;
  }

  /**
   * Validate `sequence` against the next expected inbound sequence WITHOUT
   * advancing the expectation (auth-before-advance — contract §7 amendment:
   * the inbound sequence position is checked at wire position, but the
   * counter only advances after envelope-auth verify passes). Throws
   * `inbound_sequence_mismatch` on mismatch and leaves the expectation
   * unchanged, exactly like `advance`.
   */
  peek(sequence: number): void {
    if (sequence !== this.nextExpectedValue) {
      throw new CoreInvokeError(
        "inbound_sequence_mismatch",
        `inbound sequence ${sequence} is not the next expected ${this.nextExpectedValue}`,
        { expected: this.nextExpectedValue, actual: sequence },
      );
    }
  }

  nextExpected(): number {
    return this.nextExpectedValue;
  }
}
