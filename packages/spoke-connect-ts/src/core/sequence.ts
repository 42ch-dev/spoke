/**
 * Per-direction session sequence counters (pure; no atomics).
 *
 * Ported from `crates/spoke-connect/src/core/sequence.rs`; normative rules
 * `.mstar/specs/spoke-connect.md` §Ordering semantics / §Session-core state
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

  /** Creates a counter starting at 0 (the first allocate returns 0). */
  constructor() {
    this.nextValue = 0;
  }

  /**
   * Assign the next outbound sequence.
   *
   * The counter starts at 0; on exhaustion (past the JSON-safe wire maximum)
   * `sequence_exhausted` is thrown and the counter is left exhausted —
   * sequences never wrap. The caller must close the session.
   */
  allocate(): number {
    const sequence = this.nextValue;
    if (sequence > MAX_SEQUENCE) {
      throw new CoreInvokeError("sequence_exhausted");
    }
    this.nextValue = sequence + 1;
    return sequence;
  }

  /** The next sequence that will be assigned by `allocate`. */
  next(): number {
    return this.nextValue;
  }

  /**
   * Test-only: position the counter at `value` so transport tests can
   * exercise exhaustion without 2⁵³ allocations (mirrors Rust
   * `#[cfg(test)] set_next`).
   *
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

  /** Creates an expectation starting at 0 (the first accepted sequence). */
  constructor() {
    this.nextExpectedValue = 0;
  }

  /**
   * Accept `sequence` iff it equals the next expected inbound sequence; on
   * acceptance the expectation advances by 1 and the new expectation is
   * returned. A replayed or out-of-order sequence throws
   * `inbound_sequence_mismatch` and the expectation is left unchanged — the
   * caller must reject the invoke without dispatching it.
   */
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

  /** The next inbound sequence that will be accepted. */
  nextExpected(): number {
    return this.nextExpectedValue;
  }
}
