/**
 * Op dispatch gate: required capability ⊆ negotiated capabilities.
 *
 * Ported from `crates/spoke-connect/src/core/dispatch.rs`; normative rule
 * `.mstar/specs/spoke-connect.md` §Op dispatch gate (MUST): before executing
 * or forwarding a `ConnectInvokeRequest`, a host that performs op dispatch
 * MUST ensure the capability required by `op` is present in the session's
 * `negotiated_capabilities`. If absent, the host MUST NOT run the op handler
 * and MUST answer with an error branch (e.g. `op_unsupported`) — no side
 * effects.
 *
 * The gate is evaluated against `negotiated_capabilities`, not the remote
 * manifest alone and not unsigned hello `extensions`.
 */

/** Baseline capability: required by the baseline core ops. */
export const CAPABILITY_SPOKE_BASELINE = "spoke-baseline";

/** Optional capability: required by the compute-family core ops. */
export const CAPABILITY_L2_COMPUTABLE = "l2-computable";

/**
 * The minimum capability required to dispatch `op`, per the protocol v1
 * core-op table. Product-defined `op` values return `undefined` — their
 * required capability is documented by the product and configured outside
 * the core table.
 */
export function requiredCapability(op: string): string | undefined {
  switch (op) {
    case "upsert":
    case "promote":
    case "relate":
    case "check":
    case "assemble":
      return CAPABILITY_SPOKE_BASELINE;
    case "project":
    case "compute":
      return CAPABILITY_L2_COMPUTABLE;
    default:
      return undefined;
  }
}

/**
 * Whether `op` may be dispatched in a session with `negotiatedCapabilities`.
 *
 * Fails closed: an unknown `op` has no core-table requirement and is not
 * authorized by this gate (hosts answer `op_unsupported`).
 */
export function dispatchAllowed(op: string, negotiatedCapabilities: readonly string[]): boolean {
  const required = requiredCapability(op);
  if (required === undefined) {
    return false;
  }
  return negotiatedCapabilities.includes(required);
}
