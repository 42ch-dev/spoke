/**
 * note: the `spoke-history` serving face (spoke-history contract §Connect
 * operations / §9.1). `RemoteHistoryService` is the invoke seam the
 * responder calls for both `history.*` ops; `HistoryPortService` adapts a
 * provider `HistoryPort` to that face, mirroring `RemoteMemoryService` /
 * `MemoryPortService` (Rust parity: `crates/spoke-connect/src/remote/
 * serve_ports.rs`).
 *
 * The service owns the three concerns the contract keeps OUT of
 * `HistoryPort`: peer admission (`HistoryAdmission` maps the authenticated
 * `peer_id` to the `Principal` and admitted `Scope`), effective-scope
 * resolution (a present request `scope` MUST equal the admission; absent
 * uses the admission), and prune deduplication (the recorded-outcome model
 * keyed `(effective_scope, requester_principal_id, op, idempotency_key)`).
 */

import canonicalize from "canonicalize";

import type { Principal, RevisionSnapshot, Scope } from "@42ch/spoke-schemas";
import {
  jcsSha256Hex,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type HistoryPort,
  type SpokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

import { isJsonObject, validateDeclaredScope } from "./responder.js";
import { isValidRevisionSnapshot } from "./payload.js";

/**
 * Admission context: maps the authenticated connect peer to the memory-model
 * `Principal` and the `Scope` the provider admits. Both come from the host's
 * admission context, never from the payload (MEM-R003; spoke-history
 * contract §Authorization). The responder calls it inside the local
 * serve-wait budget: a host resolver MUST settle within that budget on its
 * own.
 */
export type HistoryAdmission = (
  peerId: string,
) => SpokeResult<{ principal: Principal; scope: Scope }>;

/** note: `history.revisions.get` wire op. */
export const HISTORY_OP_GET = "history.revisions.get";
/** note: `history.revisions.prune` wire op. */
export const HISTORY_OP_PRUNE = "history.revisions.prune";

/**
 * One decoded `history.*` dispatch: the validated arguments produced by
 * `decodeHistoryCall` at the service boundary — a malformed payload answers
 * `INVALID_INPUT` before admission, dedup, and any port call (Rust parity:
 * `decode_history_call`). The prune op keeps its `idempotency_key`: it is a
 * connect-layer dedup concern and never reaches the port (spoke-history
 * contract).
 */
export type HistoryCall =
  | {
      readonly kind: "get";
      readonly scope: Scope | undefined;
      readonly entityKind: string;
      readonly entityId: string;
      readonly revision: number;
    }
  | {
      readonly kind: "prune";
      readonly scope: Scope | undefined;
      readonly entityKind: string;
      readonly entityId: string;
      readonly keep: number;
      readonly idempotencyKey: string;
    };

function historyInputReject(op: string, detail: string): SpokeReject {
  return spokeReject(
    SpokeRejectCode.INVALID_INPUT,
    `invalid ${op} payload: ${detail}`,
    { op },
  );
}

/** note: `revision` / `keep` are `u64` on the wire — JS must refuse values it cannot represent exactly, never round (spoke-history contract §HistoryPort). */
function isExactNonNegativeInteger(value: unknown): value is number {
  return (
    typeof value === "number" && Number.isSafeInteger(value) && value >= 0
  );
}

/**
 * Decode the request's optional `scope` member as the shared `Scope`:
 * absent is `undefined` (the admission supplies the effective scope), a
 * present malformed member is `INVALID_INPUT` (`validateDeclaredScope` —
 * the same structural decode the knowledge ops apply to a declared Scope).
 */
function decodeHistoryScope(
  op: string,
  payload: Record<string, unknown>,
): SpokeResult<Scope | undefined> {
  if (!("scope" in payload) || payload.scope === undefined) {
    return spokeOk<Scope | undefined>(undefined);
  }
  const scope = payload.scope;
  if (!isJsonObject(scope)) {
    return historyInputReject(op, "scope must be an object");
  }
  const scopeError = validateDeclaredScope(scope);
  if (scopeError !== null) {
    return historyInputReject(op, scopeError);
  }
  return spokeOk(scope as unknown as Scope);
}

/** note: closed request shapes (Rust `deny_unknown_fields` parity). */
const HISTORY_GET_WIRE_KEYS = new Set([
  "entity_kind",
  "entity_id",
  "revision",
  "scope",
  "extensions",
]);
const HISTORY_PRUNE_WIRE_KEYS = new Set([
  "entity_kind",
  "entity_id",
  "keep",
  "idempotency_key",
  "scope",
  "extensions",
]);

const EXTENSION_MAP_KEY_PATTERN = /^[a-z][a-z0-9_-]*$/;

function isValidExtensionMap(value: unknown): boolean {
  if (!isJsonObject(value)) {
    return false;
  }
  for (const [key, namespaceValue] of Object.entries(value)) {
    if (!EXTENSION_MAP_KEY_PATTERN.test(key) || !isJsonObject(namespaceValue)) {
      return false;
    }
  }
  return true;
}

/**
 * Decode and validate one `history.*` op payload against the closed
 * request schema — undeclared top-level members, empty selectors,
 * out-of-range integers and malformed keys answer `INVALID_INPUT` at the
 * receiving boundary, before admission, dedup, and any port call
 * (Rust parity: the generated `HistoryRevisions*Request` decoders).
 */
export function decodeHistoryCall(
  op: string,
  payload: unknown,
): SpokeResult<HistoryCall> {
  if (!isJsonObject(payload)) {
    return historyInputReject(op, "payload must be an object");
  }
  const allowedKeys =
    op === HISTORY_OP_GET
      ? HISTORY_GET_WIRE_KEYS
      : op === HISTORY_OP_PRUNE
        ? HISTORY_PRUNE_WIRE_KEYS
        : undefined;
  if (allowedKeys === undefined) {
    return spokeReject(
      SpokeRejectCode.CAPABILITY_PORT_MISSING,
      `unimplemented history op ${op}`,
    );
  }
  for (const key of Object.keys(payload)) {
    if (!allowedKeys.has(key)) {
      return historyInputReject(op, `unknown property \`${key}\``);
    }
  }
  if (
    typeof payload.entity_kind !== "string" ||
    payload.entity_kind.length === 0 ||
    typeof payload.entity_id !== "string" ||
    payload.entity_id.length === 0
  ) {
    return historyInputReject(
      op,
      "entity_kind and entity_id must be non-empty strings",
    );
  }
  if (
    "extensions" in payload &&
    payload.extensions !== undefined &&
    !isValidExtensionMap(payload.extensions)
  ) {
    return historyInputReject(
      op,
      "extensions must be an ExtensionMap object when present",
    );
  }
  const scopeResult = decodeHistoryScope(op, payload);
  if (!scopeResult.ok) {
    return scopeResult;
  }
  const { entity_kind: entityKind, entity_id: entityId } = payload;
  const scope = scopeResult.value;
  switch (op) {
    case HISTORY_OP_GET: {
      if (!isExactNonNegativeInteger(payload.revision)) {
        return historyInputReject(
          op,
          "revision must be a non-negative integer the runtime can represent exactly",
        );
      }
      return spokeOk({
        kind: "get",
        scope,
        entityKind,
        entityId,
        revision: payload.revision,
      });
    }
    case HISTORY_OP_PRUNE: {
      if (!isExactNonNegativeInteger(payload.keep)) {
        return historyInputReject(
          op,
          "keep must be a non-negative integer the runtime can represent exactly",
        );
      }
      const key = payload.idempotency_key;
      // note: the wire bound is 1–128 Unicode code points (schema
      // `maxLength`; Rust parity: the generated parser's
      // `chars().count()`), not UTF-16 code units — supplementary-plane
      // keys that fit the schema bound must be admitted.
      if (
        typeof key !== "string" ||
        [...key].length === 0 ||
        [...key].length > 128
      ) {
        return historyInputReject(
          op,
          "idempotency_key is required and must be a 1–128 character string",
        );
      }
      return spokeOk({
        kind: "prune",
        scope,
        entityKind,
        entityId,
        keep: payload.keep,
        idempotencyKey: key,
      });
    }
    default:
      // note: unreachable — the allowedKeys guard rejected non-history ops;
      // kept so the switch is exhaustive.
      return spokeReject(
        SpokeRejectCode.CAPABILITY_PORT_MISSING,
        `unimplemented history op ${op}`,
      );
  }
}
/**
 * Remote-only history service seam (spoke-history contract §Connect
 * operations): the serving face the responder calls for the two
 * `history.*` ops. One callable serves both ops — `payload` is the op's
 * request object and the returned value is its response object — so the
 * service owns admission, effective-scope resolution and prune dedup
 * behind one boundary, mirroring [`RemoteMemoryService`]. Attach a
 * provider `HistoryPort` via `HistoryPortService`.
 */
export interface RemoteHistoryService {
  invoke(
    peerId: string,
    op: string,
    payload: unknown,
  ): Promise<SpokeResult<unknown>>;
}

/**
 * The prune dedup store (spoke-history contract §9.1), keyed
 * `(effective_scope, requester_principal_id, op, idempotency_key)` and
 * recording `{payload_digest, outcome}`. Host-injectable for durable
 * persistence — entries MUST outlive a Connect session and the
 * principal-bound namespace MUST stay stable across reconnects, which an
 * in-memory default cannot promise; `HistoryDedupStore` implementations
 * are responsible for that durability.
 */
export interface HistoryDedupStore {
  /**
   * Resolve one dedup key: `unseen` executes, `evicted` answers
   * `RECEIPT_EXPIRED` (a recorded outcome the store discarded must NEVER
   * silently re-execute), and `found` compares digests.
   */
  lookup(key: string): Promise<
    | { status: "unseen" }
    | { status: "evicted" }
    | { status: "found"; payloadDigest: string; outcome: unknown }
  >;
  /** Persist the digest and settled outcome for one executed key. */
  record(key: string, payloadDigest: string, outcome: unknown): Promise<void>;
}

/**
 * Bounded in-memory dedup store — the default when the host injects
 * nothing (spoke-history contract §9.1: "the default in-memory store is
 * bounded by an eviction policy the host MAY configure").
 *
 * Eviction keeps a tombstone so a replayed evicted key answers
 * `RECEIPT_EXPIRED` instead of re-executing. Tombstones are NOT subject
 * to capacity eviction (spoke-history contract §9.1): they are retained
 * for the lifetime of the store, so a replayed evicted key answers
 * `RECEIPT_EXPIRED` forever — never `unseen`.
 *
 * shortcut: tombstones grow unboundedly with distinct evicted keys; a
 * durable host store owns the bounded-retention guarantee.
 */
export class InMemoryHistoryDedupStore implements HistoryDedupStore {
  readonly #maxEntries: number;
  readonly #records = new Map<
    string,
    { payloadDigest: string; outcome: unknown }
  >();
  readonly #tombstones = new Map<string, true>();

  constructor(options?: { maxEntries?: number }) {
    this.#maxEntries = Math.max(1, options?.maxEntries ?? 1024);
  }

  async lookup(key: string): Promise<
    | { status: "unseen" }
    | { status: "evicted" }
    | { status: "found"; payloadDigest: string; outcome: unknown }
  > {
    const record = this.#records.get(key);
    if (record !== undefined) {
      return {
        status: "found",
        payloadDigest: record.payloadDigest,
        outcome: record.outcome,
      };
    }
    return this.#tombstones.has(key)
      ? { status: "evicted" }
      : { status: "unseen" };
  }

  async record(
    key: string,
    payloadDigest: string,
    outcome: unknown,
  ): Promise<void> {
    this.#records.set(key, { payloadDigest, outcome });
    this.#tombstones.delete(key);
    while (this.#records.size > this.#maxEntries) {
      const oldest = this.#records.keys().next().value;
      if (oldest === undefined) {
        break;
      }
      this.#records.delete(oldest);
      this.#tombstones.set(oldest, true);
    }
  }
}

/**
 * `RemoteHistoryService` over a provider `HistoryPort` (spoke-history
 * contract §Connect operations): admission maps the authenticated peer to
 * its `Principal` + admitted `Scope`; a present request `scope` MUST equal
 * the admission (get answers `NOT_FOUND`, prune the entity's existing
 * write-authorization denial, both without invoking the port); the
 * admitted scope is the effective scope for every port call. Prune runs
 * the §9.1 recorded-outcome dedup before the admitted call reaches the
 * port, and the service validates provider success values before emitting
 * them — a malformed provider result answers `INTERNAL_ERROR`
 * `details.kind = "transport"`, never a success.
 */
export class HistoryPortService implements RemoteHistoryService {
  readonly #port: HistoryPort;
  readonly #admission: HistoryAdmission;
  readonly #dedup: HistoryDedupStore;
  /**
   * note: same-key serialization (Rust parity: `dedup_lock` — one mutex
   * covers lookup → port call → record so two overlapping same-key
   * prunes cannot both observe `unseen` and execute twice). One FIFO
   * promise chain covers the whole dedup section; serialization lives
   * here in the service, not in the store.
   */
  #dedupQueue: Promise<void> = Promise.resolve();

  constructor(
    port: HistoryPort,
    admission: HistoryAdmission,
    options?: { dedupStore?: HistoryDedupStore },
  ) {
    this.#port = port;
    this.#admission = admission;
    this.#dedup = options?.dedupStore ?? new InMemoryHistoryDedupStore();
  }

  async invoke(
    peerId: string,
    op: string,
    payload: unknown,
  ): Promise<SpokeResult<unknown>> {
    const admitted = this.#admission(peerId);
    if (!admitted.ok) {
      return admitted;
    }
    const decoded = decodeHistoryCall(op, payload);
    if (!decoded.ok) {
      return decoded;
    }
    const call = decoded.value;
    const { principal, scope: admittedScope } = admitted.value;
    // note: a present request scope MUST equal the admitted scope (contract
    // §Connect operations); equality is membership-independent — canonical
    // JSON — because the contract admits no subset semantics for history.
    if (call.scope !== undefined) {
      const scopeEqual =
        canonicalize(call.scope) === canonicalize(admittedScope);
      if (!scopeEqual) {
        return call.kind === "get"
          ? spokeReject(
              SpokeRejectCode.NOT_FOUND,
              "history entity not found in the admitted scope",
            )
          : spokeReject(
              SpokeRejectCode.NOT_AUTHORIZED,
              "history prune denied: scope outside admission",
            );
      }
    }
    const effectiveScope = admittedScope;
    if (call.kind === "get") {
      const result = await this.#port.getHistorySnapshot(
        principal,
        effectiveScope,
        call.entityKind,
        call.entityId,
        call.revision,
      );
      if (!result.ok) {
        return result;
      }
      // note: serving-side success validation (contract §9: the serving face
      // validates provider output before emitting).
      return validServedSnapshot(result.value, call);
    }
    return this.#invokePrune(principal, effectiveScope, op, payload, call);
  }

  async #invokePrune(
    principal: Principal,
    effectiveScope: Scope,
    op: string,
    payload: unknown,
    call: Extract<HistoryCall, { kind: "prune" }>,
  ): Promise<SpokeResult<unknown>> {
    const digestResult = jcsSha256Hex(payload);
    if (!digestResult.ok) {
      return digestResult;
    }
    const scopeDigest = jcsSha256Hex(effectiveScope);
    if (!scopeDigest.ok) {
      return scopeDigest;
    }
    const dedupKey = `${scopeDigest.value}|${principal.principal_id}|${op}|${call.idempotencyKey}`;
    // note: one FIFO queue covers lookup → port call → record (Rust
    // parity: `dedup_lock`) — overlapping same-key prunes cannot both
    // observe `unseen` and execute twice.
    const settled = this.#dedupQueue.then(() =>
      this.#dedupPrune(principal, effectiveScope, call, dedupKey, digestResult.value),
    );
    // The chain only sequences: keep it alive even if plumbing throws —
    // a task's own result flows through `settled` either way.
    this.#dedupQueue = settled.then(
      () => undefined,
      () => undefined,
    );
    return settled;
  }

  async #dedupPrune(
    principal: Principal,
    effectiveScope: Scope,
    call: Extract<HistoryCall, { kind: "prune" }>,
    dedupKey: string,
    payloadDigest: string,
  ): Promise<SpokeResult<unknown>> {
    const existing = await this.#dedup.lookup(dedupKey);
    if (existing.status === "evicted") {
      return spokeReject(
        SpokeRejectCode.RECEIPT_EXPIRED,
        "prune idempotency record was evicted; the recorded outcome is no longer available and the prune is not re-executed",
      );
    }
    if (existing.status === "found") {
      if (existing.payloadDigest !== payloadDigest) {
        return spokeReject(
          SpokeRejectCode.IDEMPOTENCY_CONFLICT,
          "idempotency_key replayed with a different payload digest",
        );
      }
      // note: same-key same-digest replay returns the recorded outcome —
      // the settled SpokeResult verbatim, no port call, no re-evaluated
      // preconditions (contract §9.1).
      return existing.outcome as SpokeResult<unknown>;
    }
    const result = await this.#port.pruneHistory(
      principal,
      effectiveScope,
      call.entityKind,
      call.entityId,
      call.keep,
    );
    // note: validate the provider's success — a malformed prune result
    // is a transport decode reject, never `Ok` (contract §9). Either
    // settled outcome — refusal, validated success, or the malformed-
    // success reject — is recorded: a same-digest replay returns the
    // recorded outcome verbatim and never re-executes (contract §9.1).
    const outcome = result.ok ? validServedPruneResult(result.value) : result;
    await this.#dedup.record(dedupKey, payloadDigest, outcome);
    return outcome;
  }
}

/** note: served-side snapshot validation — the emitted get response is `{ snapshot }` per the `HistoryRevisionsGetResponse` Variant0 member. */
function validServedSnapshot(
  value: RevisionSnapshot,
  call: Extract<HistoryCall, { kind: "get" }>,
): SpokeResult<unknown> {
  // note: one strict `RevisionSnapshot` rule for both boundaries
  // (`isValidRevisionSnapshot` — Rust parity: `validate_history_snapshot`):
  // complete core shape, variant agreeing with `entity_kind`, nested
  // entity id and revision equal to the envelope, and the envelope equal
  // to the request selectors. Malformed or wrong-target → transport
  // decode reject, never `Ok`.
  if (
    !isValidRevisionSnapshot(value, {
      entity_kind: call.entityKind,
      entity_id: call.entityId,
      revision: call.revision,
    })
  ) {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "provider returned a malformed or wrong-target RevisionSnapshot",
      { kind: "transport" },
    );
  }
  return spokeOk({ snapshot: value });
}

/** note: served-side prune validation — `deleted` integer ≥ 0 and `floor_revision` present (nullable) or the answer is a decode error, never `Ok` (contract §9). */
function validServedPruneResult(value: unknown): SpokeResult<unknown> {
  if (
    !isJsonObject(value) ||
    !isExactNonNegativeInteger(value.deleted) ||
    !("floor_revision" in value) ||
    (value.floor_revision !== null &&
      !isExactNonNegativeInteger(value.floor_revision))
  ) {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "provider returned a malformed HistoryPruneResult",
      { kind: "transport" },
    );
  }
  return spokeOk({
    deleted: value.deleted,
    floor_revision: value.floor_revision,
  });
}
