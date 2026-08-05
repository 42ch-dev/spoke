//! Per-direction session sequence counters (pure; no atomics).
//!
//! Normative rules (`.mstar/specs/spoke-connect.md` §Ordering semantics,
//! §Session-core state machine): each peer maintains its **own** outbound
//! counter per session starting at 0; inbound invokes are accepted iff
//! `sequence == next_expected_inbound` (start 0) and then advance by 1.
//! When the next outbound sequence would exceed 2⁵³−1 (the JSON-safe wire
//! maximum), the session MUST be closed and reopened — **no wrap-around**.
//!
//! These are pure single-threaded counters. A transport that needs
//! concurrent allocation (concurrent invokes) wraps them in its own
//! synchronization (the current node uses an atomic around the same rules).

/// Maximum outbound invoke sequence per session: 2⁵³−1, the JSON-safe wire
/// maximum for `ConnectInvokeRequest.sequence`.
pub const MAX_SEQUENCE: u64 = (1 << 53) - 1;

use crate::core::error::CoreInvokeError;

/// Outbound sequence counter, starting at 0.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OutboundSequence {
    next: u64,
}

impl OutboundSequence {
    /// Creates a counter starting at 0 (the first allocate returns 0).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically-in-spirit assign the next outbound sequence.
    ///
    /// The counter starts at 0; on exhaustion (past the JSON-safe wire
    /// maximum) `SequenceExhausted` is returned and the counter is left
    /// exhausted — sequences never wrap. The caller must close the session.
    pub fn allocate(&mut self) -> Result<u64, CoreInvokeError> {
        let sequence = self.next;
        if sequence > MAX_SEQUENCE {
            return Err(CoreInvokeError::SequenceExhausted);
        }
        self.next += 1;
        Ok(sequence)
    }

    /// The next sequence that will be assigned by [`Self::allocate`].
    #[must_use]
    pub fn next(&self) -> u64 {
        self.next
    }

    /// Test-only: position the counter at `next` so transport tests can
    /// exercise exhaustion without 2⁵³ allocations.
    #[cfg(test)]
    pub(crate) fn set_next(&mut self, next: u64) {
        self.next = next;
    }
}

/// Inbound sequence expectation (receiver side), starting at 0.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InboundSequence {
    next_expected: u64,
}

impl InboundSequence {
    /// Creates an expectation starting at 0 (the first accepted sequence).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Accept `sequence` iff it equals the next expected inbound sequence;
    /// on acceptance the expectation advances by 1 and the new expectation
    /// is returned. A replayed or out-of-order sequence yields
    /// `InboundSequenceMismatch` and the expectation is left unchanged — the
    /// caller must reject the invoke without dispatching it.
    pub fn advance(&mut self, sequence: i64) -> Result<u64, CoreInvokeError> {
        if sequence < 0 {
            return Err(CoreInvokeError::InboundSequenceMismatch {
                expected: self.next_expected,
                actual: sequence,
            });
        }
        let sequence = sequence as u64;
        if sequence != self.next_expected {
            return Err(CoreInvokeError::InboundSequenceMismatch {
                expected: self.next_expected,
                actual: sequence as i64,
            });
        }
        self.next_expected += 1;
        Ok(self.next_expected)
    }

    /// Validate `sequence` against the next expected inbound sequence WITHOUT
    /// advancing the expectation (auth-before-advance — contract §7
    /// amendment: the inbound sequence position is checked at wire position,
    /// but the counter only advances after envelope-auth verify passes).
    /// Rejects exactly like [`Self::advance`] and leaves the expectation
    /// unchanged, so a bogus-signature envelope cannot desync the session.
    pub fn peek(&self, sequence: i64) -> Result<(), CoreInvokeError> {
        if sequence < 0 {
            return Err(CoreInvokeError::InboundSequenceMismatch {
                expected: self.next_expected,
                actual: sequence,
            });
        }
        let sequence = sequence as u64;
        if sequence != self.next_expected {
            return Err(CoreInvokeError::InboundSequenceMismatch {
                expected: self.next_expected,
                actual: sequence as i64,
            });
        }
        Ok(())
    }

    /// The next inbound sequence that will be accepted.
    #[must_use]
    pub fn next_expected(&self) -> u64 {
        self.next_expected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_starts_at_zero_and_increments_monotonically() {
        let mut seq = OutboundSequence::new();
        assert_eq!(seq.allocate().expect("first"), 0);
        assert_eq!(seq.allocate().expect("second"), 1);
        assert_eq!(seq.allocate().expect("third"), 2);
        assert_eq!(seq.next(), 3);
    }

    #[test]
    fn outbound_exhaustion_errors_without_wrapping() {
        let mut seq = OutboundSequence::new();
        seq.next = MAX_SEQUENCE;
        assert_eq!(seq.allocate().expect("last valid"), MAX_SEQUENCE);
        let err = seq.allocate().expect_err("exhausted");
        assert!(matches!(err, CoreInvokeError::SequenceExhausted));
        // Still exhausted — no wrap-around.
        let err = seq.allocate().expect_err("still exhausted");
        assert!(matches!(err, CoreInvokeError::SequenceExhausted));
    }

    #[test]
    fn inbound_accepts_sequential_sequences() {
        let mut inbound = InboundSequence::new();
        assert_eq!(inbound.advance(0).expect("first"), 1);
        assert_eq!(inbound.advance(1).expect("second"), 2);
        assert_eq!(inbound.advance(2).expect("third"), 3);
        assert_eq!(inbound.next_expected(), 3);
    }

    #[test]
    fn inbound_rejects_replay_and_out_of_order() {
        let mut inbound = InboundSequence::new();
        assert_eq!(inbound.advance(0).expect("first"), 1);

        // Replay of an already-consumed sequence.
        let err = inbound.advance(0).expect_err("replay");
        assert!(matches!(
            err,
            CoreInvokeError::InboundSequenceMismatch {
                expected: 1,
                actual: 0
            }
        ));

        // Out-of-order (skipping ahead).
        let err = inbound.advance(2).expect_err("out of order");
        assert!(matches!(
            err,
            CoreInvokeError::InboundSequenceMismatch {
                expected: 1,
                actual: 2
            }
        ));

        // The expectation is unchanged after rejections.
        assert_eq!(inbound.next_expected(), 1);
    }

    #[test]
    fn inbound_rejects_negative_sequences() {
        let mut inbound = InboundSequence::new();
        let err = inbound.advance(-1).expect_err("negative");
        assert!(matches!(
            err,
            CoreInvokeError::InboundSequenceMismatch {
                expected: 0,
                actual: -1
            }
        ));
    }

    #[test]
    fn inbound_peek_validates_without_advancing() {
        let mut inbound = InboundSequence::new();

        // Wire-position check passes at 0...
        inbound.peek(0).expect("peek at the next expected sequence");
        // ...but the expectation is unchanged — the counter only advances
        // after envelope-auth verify passes (contract §7 amendment).
        assert_eq!(inbound.next_expected(), 0);

        // Mismatch rejects exactly like advance, and still does not advance.
        let err = inbound.peek(1).expect_err("peek ahead");
        assert!(matches!(
            err,
            CoreInvokeError::InboundSequenceMismatch {
                expected: 0,
                actual: 1
            }
        ));
        let err = inbound.peek(-1).expect_err("peek negative");
        assert!(matches!(
            err,
            CoreInvokeError::InboundSequenceMismatch {
                expected: 0,
                actual: -1
            }
        ));
        assert_eq!(inbound.next_expected(), 0);

        // After a real advance, peek tracks the new expectation.
        assert_eq!(inbound.advance(0).expect("advance"), 1);
        inbound.peek(1).expect("peek at the next expected sequence");
        assert_eq!(inbound.next_expected(), 1);
    }

    #[test]
    fn inbound_rejects_sequences_above_the_wire_maximum() {
        let mut inbound = InboundSequence::new();
        let err = inbound
            .advance((MAX_SEQUENCE + 1) as i64)
            .expect_err("above wire max");
        assert!(matches!(
            err,
            CoreInvokeError::InboundSequenceMismatch { .. }
        ));
    }
}
