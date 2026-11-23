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

import type {
  KnowledgeAdoptRequest,
  Principal,
  RevisionSnapshot,
  Scope,
} from "@42ch/spoke-schemas";
import {
  jcsSha256Hex,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type AdoptionPort,
  type HistoryPort,
  type SpokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

import { isJsonObject, validateDeclaredScope } from "./responder.js";
import {
  isValidExtensionMap,
  isValidKnowledgeEntryDeep,
  isValidRevisionSnapshot,
  isWellFormedAdoptionAnchor,
} from "./payload.js";

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
 * The adoption exact-integer rule (spoke-knowledge-adoption contract,
 * canonical numeric spelling): every number member validates by its
 * parsed VALUE — an integral value is admissible exactly when its
 * magnitude is within the inclusive exact-integer bound `2^53`
 * (`9007199254740993` parses to exactly `2^53` and is the same wire
 * number JavaScript admits) or beyond the u64 range (the `≥ 2^64`
 * hinge where Rust `serde_jcs` also prints an f64). Deliberately
 * separate from the history digest predicate in Rust's
 * `payload_integers_jcs_exact`, whose stricter `2^53 − 1` ceiling
 * refuses an unrepresentable integer literal (Rust parity:
 * `adopt_payload_integers_exact`; qc1 F-013 / qc3 F-006).
 */
const ADOPT_EXACT_INTEGER_BOUND = 9_007_199_254_740_992; // 2^53, inclusive
const U64_RANGE_BOUND = 18_446_744_073_709_551_616; // 2^64

function adoptPayloadIntegersExact(value: unknown): boolean {
  if (typeof value === "number") {
    if (!Number.isInteger(value)) {
      return true;
    }
    return (
      Math.abs(value) <= ADOPT_EXACT_INTEGER_BOUND ||
      Math.abs(value) >= U64_RANGE_BOUND
    );
  }
  if (Array.isArray(value)) {
    return value.every(adoptPayloadIntegersExact);
  }
  if (isJsonObject(value)) {
    return Object.values(value).every(adoptPayloadIntegersExact);
  }
  return true;
}
/**
 * The prune dedup store (spoke-history contract §9.1), keyed
 * `(effective_scope, requester_principal_id, op, idempotency_key)` and
 * recording `{payload_digest, outcome}`. Host-injectable for durable
 * persistence — entries MUST outlive a Connect session and the
 * principal-bound namespace MUST stay stable across reconnects, which an
 * in-memory default cannot promise; `HistoryDedupStore` implementations
 * are responsible for that durability.
 *
 * `settle` additionally owns keyed single-flight (spoke-knowledge-
 * adoption contract §Repeat and idempotency): one store object shared
 * by multiple service instances still yields at most one provider
 * invocation per dedup key, through atomic probe-and-record or keyed
 * single-flight at this interface. Calls with unrelated keys proceed
 * concurrently and stay independent of an unrelated in-flight call.
 * Cross-process atomicity is a store-implementation obligation.
 *
 * `settle` is OPTIONAL (contract §Injected dedup stores): a lookup/
 * record-only store keeps compiling and inherits the serving layer's
 * process-wide keyed single-flight claim — the default claim is keyed
 * on the four-member retry identity and held in process memory
 * (`settleDedupKey`, Rust parity: `HistoryDedupStore::claim_key`'s
 * default). Implement it only for a stronger store-level exclusion
 * (e.g. cross-process atomic claim); the default is per-process.
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
  /**
   * Run `execute` with exclusive ownership of `key`: concurrent `settle`
   * calls for the SAME key never overlap (the second waits for the first
   * to record), while unrelated keys proceed independently. The
   * exclusion holds across every user of THIS store object — that is
   * what makes a shared store never double-invoke.
   */
  settle?<T>(key: string, execute: () => Promise<T>): Promise<T>;
}

/**
 * The serving layer's process-wide keyed single-flight fallback
 * (spoke-knowledge-adoption contract §Injected dedup stores): one
 * in-flight execution chain per dedup key, shared by every service that
 * consults this helper, so a lookup/record-only injected store still
 * gets same-key exclusion across service instances. A store that
 * implements `settle` overrides this default. Rust parity: the default
 * `HistoryDedupStore::claim_key`.
 */
const SHARED_DEDUP_IN_FLIGHT = new Map<string, Promise<unknown>>();

async function sharedKeyedSettle<T>(
  key: string,
  execute: () => Promise<T>,
): Promise<T> {
  const prior = SHARED_DEDUP_IN_FLIGHT.get(key);
  const current = (prior ?? Promise.resolve()).then(() => execute());
  const tail = current.then(
    () => undefined,
    () => undefined,
  );
  SHARED_DEDUP_IN_FLIGHT.set(key, tail);
  void tail.then(() => {
    // Only clear the entry when no later claim chained onto it.
    if (SHARED_DEDUP_IN_FLIGHT.get(key) === tail) {
      SHARED_DEDUP_IN_FLIGHT.delete(key);
    }
  });
  return current;
}

/**
 * Run `execute` under same-key exclusion: the store's own `settle`
 * override when it implements one, else the shared keyed single-flight
 * fallback. This is the only call the serving faces use for the
 * lookup → invoke → record critical section.
 */
function settleDedupKey<T>(
  store: HistoryDedupStore,
  key: string,
  execute: () => Promise<T>,
): Promise<T> {
  return store.settle !== undefined
    ? store.settle(key, execute)
    : sharedKeyedSettle(key, execute);
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
  /**
   * note: keyed single-flight (spoke-knowledge-adoption contract §Repeat
   * and idempotency) — one in-flight execution chain per dedup key, owned
   * by the store object itself so every service instance sharing it
   * serializes the SAME key while unrelated keys proceed concurrently.
   */
  readonly #inFlight = new Map<string, Promise<unknown>>();

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
  /**
   * Keyed single-flight: `execute` runs after the previous `settle` for
   * the same key resolves; unrelated keys run independently. The chain
   * entry is removed once the execution settles so the map stays empty
   * when nothing is in flight.
   */
  async settle<T>(key: string, execute: () => Promise<T>): Promise<T> {
    const prior = this.#inFlight.get(key);
    const current = (prior ?? Promise.resolve()).then(() => execute());
    const tail = current.then(
      () => undefined,
      () => undefined,
    );
    this.#inFlight.set(key, tail);
    void tail.then(() => {
      // Only clear the entry when no later settle chained onto it.
      if (this.#inFlight.get(key) === tail) {
        this.#inFlight.delete(key);
      }
    });
    return current;
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
    // note: the dedup critical section (lookup → port call → record)
    // runs under the store-interface keyed single-flight claim (Rust
    // parity: `dedup.claim_key`) — overlapping same-key prunes cannot
    // both observe `unseen` and execute twice, while unrelated keys
    // proceed concurrently (contract §Repeat and idempotency /
    // §Injected dedup stores).
    return settleDedupKey(this.#dedup, dedupKey, () =>
      this.#dedupPrune(
        principal,
        effectiveScope,
        call,
        dedupKey,
        digestResult.value,
      ),
    );
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

/**
 * note: the `spoke-knowledge-adoption` serving face (spoke-knowledge-
 * adoption contract §Connect operation and capability gate /
 * §Repeat and idempotency). `RemoteAdoptionService` is the invoke seam
 * the responder calls for `knowledge.adopt`; `AdoptionPortService`
 * adapts a provider `AdoptionPort` to that face, mirroring
 * `RemoteHistoryService` / `HistoryPortService` (Rust parity:
 * `crates/spoke-connect/src/remote/serve_ports.rs`).
 *
 * The service owns the two concerns the contract keeps OUT of
 * `AdoptionPort`: peer admission (`AdoptionAdmission` maps the
 * authenticated `peer_id` to the `Principal` and admitted `Scope`) and
 * connect idempotency deduplication (the `(effective_scope,
 * requester_principal_id, op, idempotency_key)` namespace with the
 * SHA-256 / RFC 8785 request digest). Retirement resolution, owner-
 * private absence, copy rules and create-only OCC all live in the
 * provider port — they never reach this seam.
 */

/**
 * Admission context: maps the authenticated connect peer to the
 * `Principal` and the `Scope` the provider admits for source visibility
 * and destination authorization (MEM-R003 parity; contract
 * §Standalone `AdoptionPort`). Both come from the host's admission
 * context, never from the payload — the request carries scope ids, not a
 * declared `Scope`.
 */
export type AdoptionAdmission = (
  peerId: string,
) => SpokeResult<{ principal: Principal; scope: Scope }>;

/** note: `knowledge.adopt` wire op. */
export const KNOWLEDGE_OP_ADOPT = "knowledge.adopt";

/** note: closed request shape (Rust `deny_unknown_fields` parity). */
const ADOPTION_ADOPT_WIRE_KEYS = new Set([
  "source_scope_id",
  "source_entry_id",
  "source_revision",
  "destination_scope_id",
  "destination_entry_id",
  "idempotency_key",
  "extensions",
]);

function adoptionInputReject(op: string, detail: string): SpokeReject {
  return spokeReject(
    SpokeRejectCode.INVALID_INPUT,
    `${op} request decode failed: ${detail}`,
  );
}

/**
 * Decode and validate one `knowledge.adopt` payload against the closed
 * `KnowledgeAdoptRequest` schema — undeclared members, empty or missing
 * selectors, a negative or non-integer `source_revision`, and a missing /
 * over-length `idempotency_key` (1–128 Unicode code points, never
 * UTF-16 code units) and an explicit `null` `destination_entry_id`
 * answer `INVALID_INPUT` at the receiving boundary,
 * before admission, dedup, and any port call (Rust parity: the generated
 * `KnowledgeAdoptRequest` decoder). The returned request keeps its
 * `idempotency_key` — the port receives the complete typed request
 * (contract §Standalone `AdoptionPort`); dedup consumes it here without
 * removing it.
 */
export function decodeAdoptionCall(
  op: string,
  payload: unknown,
): SpokeResult<KnowledgeAdoptRequest> {
  if (!isJsonObject(payload)) {
    return adoptionInputReject(op, "payload must be an object");
  }
  if (op !== KNOWLEDGE_OP_ADOPT) {
    return adoptionInputReject(op, `unknown adoption op ${op}`);
  }
  for (const key of Object.keys(payload)) {
    if (!ADOPTION_ADOPT_WIRE_KEYS.has(key)) {
      return adoptionInputReject(op, `undeclared member ${key}`);
    }
  }
  const requireNonEmptyString = (key: string): string | SpokeReject => {
    const value = payload[key];
    if (typeof value !== "string" || value.length === 0) {
      return adoptionInputReject(op, `${key} must be a non-empty string`);
    }
    return value;
  };
  const sourceScopeId = requireNonEmptyString("source_scope_id");
  if (typeof sourceScopeId !== "string") return sourceScopeId;
  const sourceEntryId = requireNonEmptyString("source_entry_id");
  if (typeof sourceEntryId !== "string") return sourceEntryId;
  const destinationScopeId = requireNonEmptyString("destination_scope_id");
  if (typeof destinationScopeId !== "string") return destinationScopeId;
  const idempotencyKey = payload.idempotency_key;
  if (typeof idempotencyKey !== "string") {
    return adoptionInputReject(op, "idempotency_key must be a string");
  }
  // why: the contract counts Unicode code points, not UTF-16 units or
  // UTF-8 bytes — spread iterates code points.
  const keyLength = [...idempotencyKey].length;
  if (keyLength < 1 || keyLength > 128) {
    return adoptionInputReject(
      op,
      "idempotency_key must be 1–128 Unicode code points",
    );
  }
  const sourceRevision = payload.source_revision;
  // note: the decode bound is the inclusive adoption endpoint `2^53`
  // (C5; qc1 F-013 / qc3 F-006) — NOT `Number.isSafeInteger`, which
  // stops one below it and would split the endpoint from Rust's
  // `adopt_request_from_payload` decode.
  if (
    typeof sourceRevision !== "number" ||
    !Number.isInteger(sourceRevision) ||
    sourceRevision < 0 ||
    sourceRevision > ADOPT_EXACT_INTEGER_BOUND
  ) {
    return adoptionInputReject(
      op,
      "source_revision must be a non-negative integer within the exact-integer range",
    );
  }
  const destinationEntryId = payload.destination_entry_id;
  if (
    destinationEntryId !== undefined &&
    (typeof destinationEntryId !== "string" || destinationEntryId.length === 0)
  ) {
    return adoptionInputReject(
      op,
      "destination_entry_id must be a non-empty string when present",
    );
  }
  if (
    payload.extensions !== undefined &&
    !isValidExtensionMap(payload.extensions)
  ) {
    return adoptionInputReject(op, "extensions must be an ExtensionMap");
  }
  return spokeOk({
    source_scope_id: sourceScopeId,
    source_entry_id: sourceEntryId,
    source_revision: sourceRevision,
    destination_scope_id: destinationScopeId,
    ...(destinationEntryId !== undefined
      ? { destination_entry_id: destinationEntryId }
      : {}),
    idempotency_key: idempotencyKey,
    ...(payload.extensions !== undefined
      ? { extensions: payload.extensions as KnowledgeAdoptRequest["extensions"] }
      : {}),
  });
}

/**
 * Remote-only adoption service seam (spoke-knowledge-adoption contract
 * §Connect operation): the serving face a responder host attaches to
 * answer `knowledge.adopt`. It owns peer admission and the connect
 * idempotency envelope — the dedup namespace and request digest never
 * reach the provider `AdoptionPort` (contract §Repeat and idempotency;
 * mirroring `RemoteHistoryService`; Rust parity:
 * `RemoteServePorts::as_adoption`).
 */
export interface RemoteAdoptionService {
  invoke(
    peerId: string,
    op: string,
    payload: unknown,
  ): Promise<SpokeResult<unknown>>;
}

/**
 * The adoption dedup store (contract §Repeat and idempotency), keyed
 * `(effective_scope, requester_principal_id, op, idempotency_key)` and
 * recording `{payload_digest, outcome}`. Host-injectable for durable
 * persistence — entries MUST outlive a Connect session and the
 * principal-bound namespace MUST stay stable across reconnects.
 * Identical shape to `HistoryDedupStore`.
 */
export type AdoptionDedupStore = HistoryDedupStore;

/**
 * `RemoteAdoptionService` over a provider `AdoptionPort`
 * (spoke-knowledge-adoption contract): admission maps the authenticated
 * peer to its `Principal` + admitted `Scope`; dedup runs the
 * recorded-outcome model before the admitted call reaches the port —
 * same key + same digest returns the recorded outcome, same key +
 * different digest refuses `IDEMPOTENCY_CONFLICT` (contract §Repeat and
 * idempotency: the MemoryPort model). The service validates provider
 * success values before emitting them: the emitted `knowledge.adopt`
 * success response is `{ outcome, entry }` per the
 * `KnowledgeAdoptResponse` success branch — a malformed provider result
 * answers `INTERNAL_ERROR` `details.kind = "transport"`, never `Ok`.
 */
export class AdoptionPortService implements RemoteAdoptionService {
  readonly #port: AdoptionPort;
  readonly #admission: AdoptionAdmission;
  readonly #dedup: HistoryDedupStore;

  constructor(
    port: AdoptionPort,
    admission: AdoptionAdmission,
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
    if (op !== KNOWLEDGE_OP_ADOPT) {
      return spokeReject(
        SpokeRejectCode.CAPABILITY_PORT_MISSING,
        `unimplemented adoption op ${op}`,
      );
    }
    // note: the digest covers the WHOLE request object under RFC 8785 —
    // the adoption exact-integer rule (inclusive 2^53 endpoint, C5) —
    // refuse any member outside it before hashing (Rust parity:
    // `adopt_payload_integers_exact`; history keeps its own stricter
    // `payloadIntegersJcsExact` rule).
    if (!adoptPayloadIntegersExact(payload)) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `invalid ${op} payload: integer member outside the exact-integer range`,
      );
    }
    // note: request validation and decode complete before admission
    // (contract §KnowledgeAdoptRequest): a malformed payload keeps its
    // `INVALID_INPUT` regardless of the admission decision, in the same
    // order as the Rust serving face.
    const decoded = decodeAdoptionCall(op, payload);
    if (!decoded.ok) {
      return decoded;
    }
    const request = decoded.value;
    const digestResult = jcsSha256Hex(payload);
    if (!digestResult.ok) {
      return digestResult;
    }
    const admitted = this.#admission(peerId);
    if (!admitted.ok) {
      return admitted;
    }
    const { principal, scope: effectiveScope } = admitted.value;
    // note: destination admission (contract Wire-wide rules / C7;
    // QC2-08): the destination `scope_id` is authorized against the
    // admitted context BEFORE dedup, single-flight and any provider or
    // source-chain work — an out-of-scope destination refuses
    // `NOT_AUTHORIZED` without touching the source (Rust parity:
    // `AdoptionPortService::invoke`).
    if (request.destination_scope_id !== effectiveScope.scope_id) {
      return spokeReject(
        SpokeRejectCode.NOT_AUTHORIZED,
        "adoption destination scope is outside the admitted context",
      );
    }
    // note: the dedup key is the RFC 8785 canonical JSON serialization of
    // the ordered retry-identity tuple `(op, effective_scope,
    // requester_principal_id, idempotency_key)` — structural member
    // boundaries, never delimiter joining, so distinct retry identities
    // always derive distinct keys (contract §Repeat and idempotency).
    // The `payload_digest` is stored in the record and compared inside —
    // same identity + different digest is the IDEMPOTENCY_CONFLICT branch.
    const dedupKey = canonicalize([
      op,
      effectiveScope,
      principal.principal_id,
      request.idempotency_key,
    ]);
    if (dedupKey === undefined) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `invalid ${op} dedup tuple: not JSON-canonicalizable`,
      );
    }
    // note: keyed single-flight is owned by the dedup store interface —
    // a shared store serializes the SAME key across every service
    // instance using it while unrelated keys proceed independently
    // (contract §Repeat and idempotency).
    return settleDedupKey(this.#dedup, dedupKey, () =>
      this.#dedupAdopt(
        principal,
        effectiveScope,
        request,
        dedupKey,
        digestResult.value,
      ),
    );
  }

  async #dedupAdopt(
    principal: Principal,
    effectiveScope: Scope,
    request: KnowledgeAdoptRequest,
    dedupKey: string,
    payloadDigest: string,
  ): Promise<SpokeResult<unknown>> {
    const existing = await this.#dedup.lookup(dedupKey);
    if (existing.status === "evicted") {
      return spokeReject(
        SpokeRejectCode.RECEIPT_EXPIRED,
        "adoption idempotency record was evicted; the recorded outcome is no longer available and the adoption is not re-executed",
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
      // no port call — but the wire `outcome` normalizes to
      // `already_present` (contract §Repeat and idempotency: "the same key
      // and same request digest return the existing copy with outcome
      // already_present"); recorded refusals replay verbatim. The replay is
      // a fresh snapshot of the record: a store lookup returns the stored
      // reference, and mutating the caller-visible result must never
      // alter what a later replay observes (qc3 F-009).
      const recorded = JSON.parse(
        JSON.stringify(existing.outcome),
      ) as SpokeResult<unknown>;
      if (
        recorded.ok &&
        isJsonObject(recorded.value) &&
        "outcome" in recorded.value
      ) {
        return spokeOk({
          ...recorded.value,
          outcome: "already_present",
        });
      }
      return recorded;
    }
    // note: the port receives the complete `KnowledgeAdoptRequest`,
    // `idempotency_key` included — dedup is Connect-owned, but the key
    // is pass-through context for the provider (contract §Standalone
    // `AdoptionPort`; Rust parity: `&request`).
    const result = await this.#port.adoptKnowledgeEntry(
      principal,
      effectiveScope,
      request,
    );

    // note: validate the provider's success — a malformed adopt response
    // is a transport decode reject, never `Ok`. Either settled outcome —
    // refusal, validated success, or the malformed-success reject — is
    // snapshotted before it is recorded: a same-digest replay returns the
    // recorded outcome verbatim and never re-executes, and a later
    // provider-side mutation of its own result object can never change
    // what a replay observes (contract §Repeat and idempotency). The
    // returned result is a SECOND snapshot: caller-visible results never
    // alias the dedup receipt record (qc3 F-009).
    const outcome = result.ok
      ? validServedAdoptResult(result.value, request.destination_entry_id)
      : (JSON.parse(JSON.stringify(result)) as SpokeResult<unknown>);
    await this.#dedup.record(dedupKey, payloadDigest, outcome);
    return JSON.parse(JSON.stringify(outcome)) as SpokeResult<unknown>;
  }
}

/**
 * note: served-side adopt-response validation — the emitted success value
 * is the `KnowledgeAdoptResponse` success branch `{ outcome, entry }`:
 * `outcome` is `"adopted"` or `"already_present"` and `entry` is the
 * destination `KnowledgeEntry` — a success entry ALWAYS carries
 * `adoption_anchor` (contract §KnowledgeAdoptRequest and response: "a
 * provider result whose entry lacks the anchor is malformed and is
 * answered as a transport/serving refusal rather than Ok"). Anything
 * else is a transport decode reject, never `Ok`. The emitted entry is
 * re-materialized through a JSON round-trip so the dedup record and the
 * replayed outcome can never observe a later provider-side mutation of
 * the provider's own object.
 */
function validServedAdoptResult(
  value: unknown,
  destinationEntryId: string | undefined,
): SpokeResult<unknown> {
  if (!isJsonObject(value)) {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "adoption provider returned a non-object response",
      { kind: "transport" },
    );
  }
  // Failure branch (`OneOf1` — contract E3): a provider error envelope
  // with an optional `extensions` member, emitted verbatim. A supplied
  // `extensions` on EITHER branch must satisfy `ExtensionMap` (namespace
  // pattern + object values) — the same rule request payloads enforce
  // (qc1 F-004 / qc3 F-021/F-023).
  if ("error" in value) {
    if (
      !isJsonObject(value.error) ||
      typeof value.error.code !== "string" ||
      typeof value.error.message !== "string" ||
      !isValidExtensionMap(value.error.extensions)
    ) {
      return spokeReject(
        SpokeRejectCode.INTERNAL_ERROR,
        "adoption provider returned a malformed error envelope",
        { kind: "transport" },
      );
    }
    if ("extensions" in value && !isValidExtensionMap(value.extensions)) {
      return spokeReject(
        SpokeRejectCode.INTERNAL_ERROR,
        "adoption provider returned a malformed extensions member",
        { kind: "transport" },
      );
    }
    const emitted: Record<string, unknown> = { error: value.error };
    if ("extensions" in value) {
      emitted.extensions = value.extensions;
    }
    return spokeOk(JSON.parse(JSON.stringify(emitted)) as unknown);
  }

  if ("extensions" in value && !isValidExtensionMap(value.extensions)) {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "adoption provider returned a malformed extensions member",
      { kind: "transport" },
    );
  }
  const outcome = value.outcome;
  if (outcome !== "adopted" && outcome !== "already_present") {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "adoption provider returned an invalid outcome member",
      { kind: "transport" },
    );
  }
  if (
    !isJsonObject(value.entry) ||
    !isValidKnowledgeEntryDeep(value.entry)
  ) {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "adoption provider returned an entry that is not a complete KnowledgeEntry",
      { kind: "transport" },
    );
  }
  if (!isWellFormedAdoptionAnchor(value.entry.adoption_anchor)) {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "adoption provider returned a success entry without a well-formed adoption_anchor",
      { kind: "transport" },
    );
  }
  // note: QC2-03 — an explicit `destination_entry_id` binds the served
  // `entry.entry_id` to the requested selector; a success entry for
  // another slot is malformed (Rust parity: `valid_served_adopt_result`).
  // An absent selector defaults to the entry's own id, so no equality
  // check applies then.
  if (
    destinationEntryId !== undefined &&
    value.entry.entry_id !== destinationEntryId
  ) {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "adoption provider returned an entry whose entry_id does not match the requested destination_entry_id",
      { kind: "transport" },
    );
  }
  // note: snapshot the settled JSON — the replay must return the
  // recorded outcome, not a live reference into provider state. A
  // producer-supplied `extensions` member survives verbatim (contract
  // E2: present-empty ≡ absent, but a supplied member is never
  // dropped).
  return spokeOk(
    JSON.parse(
      JSON.stringify(
        "extensions" in value
          ? { outcome, entry: value.entry, extensions: value.extensions }
          : { outcome, entry: value.entry },
      ),
    ) as { outcome: string; entry: unknown },
  );
}


