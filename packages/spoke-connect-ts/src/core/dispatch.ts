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
 *
 * `tools.<ns>.<tool_id>` is a **core gate rule** (normative
 * `spoke-connect.md` §Op dispatch gate): self-describing tools require the
 * op string itself — no registry, no umbrella flag. The gate then
 * evaluates that exact string against `negotiated_capabilities` as for
 * every op.
 */
export function requiredCapability(op: string): string | undefined {
  if (op.startsWith("tools.")) {
    return op;
  }
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

/**
 * Whether a capability-token grant (validated `claims.capabilities`)
 * authorizes `op` — **membership** of `op`'s required capability in the
 * grant (normative §Capability matching: subset-of-grant, not exact-list
 * equality; extra capabilities on the token are ignored when unused).
 *
 * `required` is the op's required capability from the core table or the
 * product-configured map. Fails closed: an op with no requirement is not
 * authorized by the token gate.
 *
 * Hosts compose this with `dispatchAllowed`: when a token grant is in
 * effect, both the negotiated set and the grant must allow the op (the
 * token does not replace `negotiatedCapabilities`).
 */
export function tokenAuthorizesOp(
  required: string | undefined,
  tokenCapabilities: readonly string[],
): boolean {
  return required !== undefined && tokenCapabilities.includes(required);
}
