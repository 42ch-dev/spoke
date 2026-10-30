//! SPOKE reference memory engine — deterministic, I/O-free algorithm policy.
//!
//! The engine receives already-authorized data as pure function inputs and
//! returns classification, ranking, reflection-assembly, and transfer
//! results. It holds no storage abstraction and enforces no authorization,
//! CAS, idempotency, fencing, or transaction semantics — those belong to the
//! provider behind `MemoryPort` (memory protocol §1.1).

pub mod classify;
pub mod recall;
pub mod reflect;
pub mod transfer;

/// Algorithm-policy version of this engine, surfaced by `memory.describe` and
/// recorded on each reflection's generator (memory protocol §3).
pub const ENGINE_POLICY_VERSION: &str = "spoke-memory/1.0.0-deterministic";

/// Pure-function facade over the four policy modules.
#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryEngine;
