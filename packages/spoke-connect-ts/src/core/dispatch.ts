/**
 * Op dispatch gate: required capability ⊆ negotiated capabilities.
 *
 * Ported from `crates/spoke-connect/src/core/dispatch.rs`; see
 * `.agents/specs/connect/spoke-connect.md` contract §Op dispatch gate (MUST): before executing
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

/**
 * Optional capability: required by the three remote knowledge ops
 * (`port.knowledge.get` / `port.knowledge.put` /
 * `port.scope.list_knowledge_entries`). It must be negotiated explicitly: the
 * responder serves those ops only through the knowledge face, so no other
 * flag implies this one.
 */
export const CAPABILITY_SPOKE_KNOWLEDGE = "spoke-knowledge";

/** Optional capability: required by the compute-family core ops. */
export const CAPABILITY_L2_COMPUTABLE = "l2-computable";

/** Optional capability: required by the remote `extract` core op. */
export const CAPABILITY_KE_EXTRACTION = "ke-extraction";

/**
 * Optional capability: required by the memory family core ops (SPOKE memory
 * protocol §4.1). A provider declaring either memory sub-flag MUST also
 * declare this one.
 */
export const CAPABILITY_SPOKE_MEMORY = "spoke-memory";

/** Optional memory sub-flag: required by `memory.reflections.*`. */
export const CAPABILITY_SPOKE_MEMORY_REFLECTION = "spoke-memory-reflection";

/** Optional memory sub-flag: required by `memory.exports.*` / `memory.imports.*`. */
export const CAPABILITY_SPOKE_MEMORY_TRANSFER = "spoke-memory-transfer";

/**
 * Optional capability: the `spoke-cognition` family (one flag, no subflags).
 *
 * Not an op requirement — no core-table row names it. A request that
 * *declares* a cognition requirement (see `cognitionRequirementPresent`)
 * additionally needs this flag in the negotiated set; the payload-dependent
 * conjunct lives at the dispatch boundary next to `ke-ownership`. The legacy
 * `l5-mind` flag is never read as this family.
 */
export const CAPABILITY_SPOKE_COGNITION = "spoke-cognition";

/**
 * Optional capability: required by the history family core ops
 * (`history.revisions.get` / `history.revisions.prune`, spoke-history
 * contract §Connect operations). Dispatch requires exact `spoke-history`
 * membership — neither `spoke-baseline` nor `spoke-knowledge` satisfies it.
 */
export const CAPABILITY_SPOKE_HISTORY = "spoke-history";

/**
 * Optional data/conformance capability: a host that validates or produces
 * `Judgment` envelopes advertises `spoke-judgment` in
 * `HostCapabilityManifest.capabilities` (spoke-judgment wire contract
 * §Manifest). Manifest-only — it gates NO op, so `requiredCapability` has no
 * row for it and `tools.*` ops keep their own exact-string gates.
 */
export const CAPABILITY_SPOKE_JUDGMENT = "spoke-judgment";

/**
 * Optional capability: required by `knowledge.adopt` (spoke-knowledge-
 * adoption contract §Connect operation and capability gate). The op also
 * requires the mandatory co-capability `spoke-knowledge`; a host declaring
 * `spoke-knowledge-adoption` also declares `spoke-knowledge`.
 */
export const CAPABILITY_SPOKE_KNOWLEDGE_ADOPTION = "spoke-knowledge-adoption";

/**
 * Optional capability: required by a `port.knowledge.get` / `put` request
 * that declares a `scope` (A2-D1). It is a conjunct, never a replacement
 * for the op's `spoke-knowledge` requirement, and marks a peer that honours
 * the member instead of ignoring it. A session without it answers
 * `op_unsupported` for such a request (no silent scope drop).
 */
export const CAPABILITY_SPOKE_KNOWLEDGE_SCOPED = "spoke-knowledge-scoped";

/**
 * Wire spelling of a dispatch-layer validation verdict (consumer contract
 * §2.2). The ops protocol spells the same internal code `INVALID_INPUT` on
 * its envelopes; a rejection taken at the pre-handler boundary carries this
 * lowercase form instead, beside the other dispatch codes (`op_unsupported`,
 * `capability_missing`, `auth_failed`).
 */
export const WIRE_INVALID_INPUT = "invalid_input";

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
    case "extract":
      return CAPABILITY_KE_EXTRACTION;
    case "memory.describe":
    case "memory.spaces.list":
    case "memory.spaces.create":
    case "memory.spaces.freeze":
    case "memory.spaces.unfreeze":
    case "memory.subjects.ensure":
    case "memory.subjects.get":
    case "memory.subjects.fence":
    case "memory.grants.list":
    case "memory.grants.grant":
    case "memory.grants.revoke":
    case "memory.capture":
    case "memory.candidates.list":
    case "memory.candidates.get":
    case "memory.candidates.dismiss":
    case "memory.candidates.review":
    case "memory.records.create":
    case "memory.records.get":
    case "memory.records.update":
    case "memory.records.delete":
    case "memory.records.list":
    case "memory.records.promote":
    case "memory.recall":
    case "memory.profile.get":
    case "memory.profile.update":
    case "memory.receipts.get":
      return CAPABILITY_SPOKE_MEMORY;
    case "memory.reflections.state":
    case "memory.reflections.list":
    case "memory.reflections.generate":
      return CAPABILITY_SPOKE_MEMORY_REFLECTION;
    case "history.revisions.get":
    case "history.revisions.prune":
      return CAPABILITY_SPOKE_HISTORY;
    case "knowledge.adopt":
      return CAPABILITY_SPOKE_KNOWLEDGE_ADOPTION;
    case "memory.exports.create":
    case "memory.exports.get":
    case "memory.imports.stage":
    case "memory.imports.activate":
    case "memory.imports.discard":
      return CAPABILITY_SPOKE_MEMORY_TRANSFER;
    default:
      return undefined;
  }
}

/**
 * Whether `op` falls under a core-reserved prefix with no registered
 * row. `knowledge.*` is a reserved-product prefix owned by this core
 * decision (spoke-knowledge-adoption contract §Connect operation):
 * `knowledge.adopt` is its single registered operation — an op that
 * starts with `knowledge.` and has no core-table row is an unknown
 * reserved name, refused `op_unsupported` by every dispatch surface
 * BEFORE the product custom-map (`op_capability_requirements` and
 * equivalents) is consulted. Custom mappings apply only outside
 * reserved families. Rust parity: `reserved_prefix_op`.
 */
export function reservedPrefixOp(op: string): boolean {
  return requiredCapability(op) === undefined && op.startsWith("knowledge.");
}

/**
 * Whether `negotiated` satisfies `required` (port capability gate): plain
 * membership. No capability implies another — in particular `spoke-baseline`
 * never satisfies `spoke-knowledge`, because the knowledge ops are served
 * only through the knowledge face.
 */
export function capabilitySatisfies(
  negotiated: readonly string[],
  required: string,
): boolean {
  return negotiated.includes(required);
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
  if (!capabilitySatisfies(negotiatedCapabilities, required)) {
    return false;
  }
  if (
    required === CAPABILITY_SPOKE_KNOWLEDGE_ADOPTION &&
    !capabilitySatisfies(negotiatedCapabilities, CAPABILITY_SPOKE_KNOWLEDGE)
  ) {
    return false;
  }
  return true;
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

const SCOPE_COGNITION_OPS: Record<string, true> = {
  check: true,
  assemble: true,
  "port.scope.list_knowledge_entries": true,
  "port.scope.list_timeline_events": true,
  "port.fork.list_timeline_events": true,
  "port.knowledge.get": true,
  "port.knowledge.put": true,
  "history.revisions.get": true,
  "history.revisions.prune": true,
};

const COGNITION_FILTER_OPS: Record<string, true> = {
  "memory.recall": true,
  "memory.records.list": true,
  "memory.reflections.generate": true,
  "memory.reflections.list": true,
  "memory.reflections.state": true,
};

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * The request field path that declares a cognition requirement **for this
 * op**, or `undefined` when the request carries none. Both slots come from
 * the registered request schemas: only the actual standard carrier of the
 * dispatched operation can demand cognition, so collapsed/inspected data on
 * any other op — a tool's `arguments`, unrelated product metadata, an opaque
 * `extensions` member — stays ordinary payload data and never becomes a
 * synthetic requirement.
 *
 * Declared-location predicate only: no trimming, normalization, registry
 * lookup or recursive search. A present `null` counts as present (malformed,
 * never "absent").
 */
export function cognitionRequirementField(
  op: string,
  payload: Record<string, unknown>,
): string | undefined {
  if (Object.hasOwn(SCOPE_COGNITION_OPS, op)) {
    const scope = payload.scope;
    if (isPlainObject(scope) && scope.cognition !== undefined) {
      return "scope.cognition";
    }
  }
  if (
    Object.hasOwn(COGNITION_FILTER_OPS, op) &&
    payload.cognition_filter !== undefined
  ) {
    return "cognition_filter";
  }
  return undefined;
}

/**
 * Whether `op`'s payload declares a `spoke-cognition` requirement. The
 * signing/negotiation gate then requires `CAPABILITY_SPOKE_COGNITION` in the
 * negotiated set; ordinary payloads without the op's carrier members are
 * untouched.
 */
export function cognitionRequirementPresent(
  op: string,
  payload: Record<string, unknown>,
): boolean {
  return cognitionRequirementField(op, payload) !== undefined;
}
