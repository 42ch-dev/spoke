/**
 * note: `spoke-knowledge-adoption` conformance — corpus interpreter for
 * the adoption serving face (spoke-knowledge-adoption contract §Connect
 * operation / §Repeat and idempotency / §Anchor immutability). Loads the
 * shared wire vectors from `fixtures/adoption-conformance/` and drives
 * them against the SHIPPED serving path — the same JSON case produces
 * the same wire outcome as the Rust runner
 * (`crates/spoke-connect/tests/adoption_conformance.rs`).
 *
 * `surface: "wire"` cases run over a real `connectResponder` ↔
 * `RemoteAdapter` loopback (the compound capability gate, responder
 * dispatch, `AdoptionPortService` decode / admission / dedup and the
 * served-response validation boundary); `surface: "service"` cases run
 * through `AdoptionPortService.invoke` directly.
 *
 * The scripted `AdoptionPort` implements only the provider-owned
 * contract semantics over the seeded tables — owner-private admission,
 * the shared `resolveRetirementChain` helper, the copy rules,
 * create-only OCC, anchor repeat classification — so every gate, dedup
 * and validation assertion measures connect code, never fixture policy.
 * The fixture scoped store applies the shared
 * `assertAdoptionAnchorPreserved` gate before OCC (the anchor-immutable
 * vectors measure that shipped helper over the wire).
 */

import { readdirSync, readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import {
  assertAdoptionAnchorPreserved,
  resolveRetirementChain,
  spokeOk,
  spokeReject,
  type AdoptionPort,
  type SpokeResult,
} from "@42ch/spoke-operations";

import type {
  HostCapabilityManifest,
  KnowledgeAdoptRequest,
  KnowledgeAdoptResponse,
  KnowledgeEntry,
  Principal,
  Scope,
} from "@42ch/spoke-schemas";

import {
  connectRemoteAdapter,
  connectResponder,
  loopbackTransportPair,
} from "../../src/remote/index.js";
import {
  AdoptionPortService,
  InMemoryHistoryDedupStore,
  type HistoryDedupStore,
  type RemoteAdapter,
  type RemoteAdoptionService,
  type RemoteServePorts,
} from "../../src/remote/index.js";
import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";

function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

function manifest(
  hostId: string,
  capabilities: string[],
): HostCapabilityManifest {
  return {
    schema_version: 1,
    host_id: hostId,
    roles: ["data-store"],
    capabilities: capabilities as [string, ...string[]],
    namespaces: ["world_a"],
    extensions: {},
  };
}

const HOST_CAPABILITIES = ["spoke-knowledge", "spoke-knowledge-adoption"];
const CLIENT_CAPABILITIES = ["spoke-knowledge", "spoke-knowledge-adoption"];
/** note: the destination scope the fixture store and knowledge face admit. */
const DESTINATION_SCOPE = "spc_local";
/** note: every source entry is stored under this scope in the source table. */
const SOURCE_SCOPE = "spc_shared";
const DEFAULT_ADMITTED_SCOPES = [SOURCE_SCOPE, DESTINATION_SCOPE];

const PRINCIPAL: Principal = { principal_id: "prn_client", extensions: {} };

// ── Scripted provider ─────────────────────────────────────────────────────

type JsonObject = Record<string, unknown>;

/** The narrowed success-entry type: `adoption_anchor` is mandatory (C3/F-005). */
type AdoptedEntry = Extract<KnowledgeAdoptResponse, { entry: unknown }>["entry"];

interface SeededSource {
  entryId: string;
  scopeId: string;
  successors: string[] | undefined;
  entry: KnowledgeEntry & JsonObject;
}

interface CaseInput {
  sources?: Array<{ entry_id: string; scope_id?: string; successors?: string[]; entry: KnowledgeEntry & JsonObject }>;
  destinations?: KnowledgeEntry[];
  admitted_viewpoint?: string;
  admitted_scopes?: string[];
  destination_governance?: JsonObject;
  client_capabilities?: string[];
  host_capabilities?: string[];
  adoption_face?: boolean;
  dedup_capacity?: number;
}

/**
 * Shared state between the scripted `AdoptionPort` and the fixture
 * scoped store: the source table (keyed `${scopeId}|${entryId}`), the
 * destination map the `RemoteKnowledgeFace` reads/writes (so the written
 * copy is observable through `port.knowledge.get`), the provider call
 * counter, and the destination governance the copy rules reset to.
 */
function fixtureState(input: CaseInput): FixtureState {
  const sources = new Map<string, SeededSource>();
  for (const source of input.sources ?? []) {
    // note: an optional per-source `scope_id` seeds the entry under a
    // scope other than `SOURCE_SCOPE` so a case can resolve a retirement
    // terminal that lives outside the admitted scopes.
    sources.set(`${source.scope_id ?? SOURCE_SCOPE}|${source.entry_id}`, {
      entryId: source.entry_id,
      scopeId: source.scope_id ?? SOURCE_SCOPE,
      successors: source.successors,
      entry: source.entry,
    });
  }
  const destinations = new Map<string, KnowledgeEntry>();
  for (const entry of input.destinations ?? []) {
    destinations.set(entry.entry_id, entry);
  }
  return {
    sources,
    destinations,
    calls: [] as string[],
    admittedScopes: input.admitted_scopes ?? DEFAULT_ADMITTED_SCOPES,
    destinationGovernance: input.destination_governance ?? {},
  };
}

/**
 * The fixture's seeded state: the source table (keyed
 * `${scopeId}|${entryId}`), the destination map the
 * `RemoteKnowledgeFace` reads/writes (so the written copy is observable
 * through `port.knowledge.get`), the provider call counter, and the
 * destination governance the copy rules reset to.
 */
interface FixtureState {
  sources: Map<string, SeededSource>;
  destinations: Map<string, KnowledgeEntry>;
  calls: string[];
  admittedScopes: string[];
  destinationGovernance: JsonObject;
}

/** note: one generic absence — missing, out-of-scope and owner-private all answer it. */
function genericNotFound<T>(): SpokeResult<T> {
  return spokeReject("NOT_FOUND", "no such source");
}

/**
 * The destination copy per the contract copy rules: body /
 * canonical_name / entry_type / status preserved; `source_anchor`
 * cleared; `modules` minus `cognition`; `extensions` minus
 * `mindcore_adoption`; owner/disclosure reset to the destination
 * governance; `adoption_anchor` records the resolved terminal triple.
 */
function copyForDestination(
  state: FixtureState,
  terminalEntry: KnowledgeEntry & JsonObject,
  terminal: { source_scope_id: string; source_entry_id: string; source_revision: number },
  destinationId: string,
  anchorRevision: number,
): AdoptedEntry & JsonObject {
  const copy: JsonObject = {};
  for (const key of [
    "schema_version",
    "entry_type",
    "canonical_name",
    "status",
    "body",
  ]) {
    if (key in terminalEntry) {
      copy[key] = (terminalEntry as JsonObject)[key];
    }
  }
  copy.entry_id = destinationId;
  copy.revision = 1;
  if (terminalEntry.modules !== undefined) {
    const modules: JsonObject = {};
    for (const [key, value] of Object.entries(
      terminalEntry.modules as JsonObject,
    )) {
      if (key !== "cognition") {
        modules[key] = value;
      }
    }
    copy.modules = modules;
  }
  const extensions: JsonObject = {};
  for (const [key, value] of Object.entries(
    (terminalEntry.extensions ?? {}) as JsonObject,
  )) {
    if (key !== "mindcore_adoption") {
      extensions[key] = value;
    }
  }
  copy.extensions = extensions;
  for (const key of ["owner", "disclosure"]) {
    if (key in state.destinationGovernance) {
      copy[key] = state.destinationGovernance[key];
    }
  }
  copy.adoption_anchor = {
    schema_version: 1,
    source_scope_id: terminal.source_scope_id,
    source_entry_id: terminal.source_entry_id,
    source_revision: anchorRevision,
    extensions: {},
  };
  return copy as AdoptedEntry & JsonObject;
}

/**
 * Scripted `AdoptionPort`: the provider-owned contract semantics over
 * the seeded tables. `calls` is the observable "did the port run" probe
 * (`provider_calls`).
 */
function scriptedAdoptionPort(state: FixtureState): AdoptionPort {
  return {
    async adoptKnowledgeEntry(principal, scope, request) {
      state.calls.push(
        `adopt:${request.source_entry_id}@${request.source_revision}`,
      );
      const sourceScopeId = request.source_scope_id;
      const sourceEntryId = request.source_entry_id;
      const destinationScopeId = request.destination_scope_id;
      const sourceRevision = request.source_revision;
      // note: C7 destination admission (D2): the destination `space_id`
      // authorization check precedes every source access — an out-of-scope
      // destination refuses `NOT_AUTHORIZED` without touching the source,
      // so a retired or unresolvable source can never leak a chain trace
      // to an unauthorized caller.
      if (!state.admittedScopes.includes(destinationScopeId)) {
        return spokeReject(
          "NOT_AUTHORIZED",
          "destination scope not authorized",
        );
      }
      // Admission: an unadmitted source scope is the same generic
      // absence as a missing entry.
      if (!state.admittedScopes.includes(sourceScopeId)) {
        return genericNotFound();
      }
      // Source load: missing or owner-private to another viewpoint is
      // one generic NOT_FOUND with no source-identifying details.
      const requested = state.sources.get(
        `${sourceScopeId}|${sourceEntryId}`,
      )?.entry;
      if (requested === undefined) {
        return genericNotFound();
      }
      if (requested.disclosure === "owner-private") {
        const owner = requested.owner as string | undefined;
        if (owner !== (scope.viewpoint as string | undefined)) {
          return genericNotFound();
        }
      }

      // Retirement-chain resolution via the shared helper (cycle /
      // ambiguous / deleted terminal / 33rd edge answers SOURCE_RETIRED
      // with details.retirement_chain).
      const chainNode = (seeded: SeededSource) => ({
        entry_id: seeded.entry.entry_id,
        scope_id: seeded.scopeId,
        status: seeded.entry.status,
        revision: seeded.entry.revision,
      });
      const terminal = await resolveRetirementChain(
        chainNode({ entryId: "", scopeId: sourceScopeId, successors: undefined, entry: requested }),
        async (node) =>
          state.sources.get(`${node.scope_id}|${node.entry_id}`)?.successors,
        async (entryId) => {
          // note: a successor id resolves against the whole seeded table —
          // successor references are storage-owned names, and a chain may
          // legitimately reach a node stored under another scope.
          for (const key of [
            `${sourceScopeId}|${entryId}`,
            ...[...state.sources.keys()].filter(
              (k) => k.endsWith(`|${entryId}`) && k !== `${sourceScopeId}|${entryId}`,
            ),
          ]) {
            const loaded = state.sources.get(key);
            if (loaded !== undefined) {
              return chainNode(loaded);
            }
          }
          return undefined;
        },
      );
      if (!terminal.ok) {
        return terminal;
      }
      const terminalSeeded = state.sources.get(
        `${terminal.value.source_scope_id}|${terminal.value.source_entry_id}`,
      ) ?? state.sources.get(
        `${sourceScopeId}|${terminal.value.source_entry_id}`,
      );
      const terminalEntry = terminalSeeded?.entry;
      if (terminalEntry === undefined) {
        return spokeReject("INTERNAL_ERROR", "resolved terminal is not stored");
      }
      // note: a resolved terminal outside the admitted scopes is the same
      // generic absence as a missing source (contract §Admission and
      // owner-private absence).
      if (!state.admittedScopes.includes(terminal.value.source_scope_id)) {
        return genericNotFound();
      }

      // note: the entry adoption actually copies must satisfy the same
      // visibility predicate as the requested source (contract
      // §Admission and owner-private absence): an owner-private resolved
      // terminal invisible to the admitted viewpoint is the same generic
      // absence and no destination write occurs.
      if (terminalEntry.disclosure === "owner-private") {
        const owner = terminalEntry.owner as string | undefined;
        if (owner !== (scope.viewpoint as string | undefined)) {
          return genericNotFound();
        }
      }

      const destinationId =
        request.destination_entry_id ?? terminal.value.source_entry_id;
      const occupant = state.destinations.get(destinationId);
      if (occupant !== undefined) {
        const anchor = occupant.adoption_anchor as
          | {
              source_scope_id: string;
              source_entry_id: string;
              source_revision: number;
            }
          | undefined;
        if (
          anchor !== undefined &&
          anchor.source_scope_id === terminal.value.source_scope_id &&
          anchor.source_entry_id === terminal.value.source_entry_id
        ) {
          // The stored copy is classified by its anchor (contract
          // §Repeat and idempotency): the repeat comparator is the
          // resolved source tuple `(source_entry_id,
          // resolved_terminal_revision)` — the same tuple the create
          // write records below. For a live source the resolved
          // terminal revision is the requested snapshot revision; for a
          // retirement-resolved source it is the terminal's own
          // revision. A differing resolved revision refuses
          // REVISION_CONFLICT with details.adopted_revision from the
          // stored anchor.
          const resolvedRevision =
            terminal.value.source_entry_id === sourceEntryId
              ? sourceRevision
              : terminal.value.source_revision;
          if (anchor.source_revision === resolvedRevision) {
            // `occupant` was filtered on `adoption_anchor !== undefined`
            // above — it satisfies the narrowed success-entry type.
            return spokeOk({
              outcome: "already_present",
              entry: occupant as AdoptedEntry,
            });
          }
          return spokeReject(
            "REVISION_CONFLICT",
            `destination ${destinationId} already anchors source revision ${anchor.source_revision}`,
            { adopted_revision: anchor.source_revision },
          );
        }
        // An unrelated occupant retains the existing create-only put
        // refusal and is never overwritten.
        return spokeReject(
          "REVISION_CONFLICT",
          `destination ${destinationId} is already occupied`,
        );
      }
      // The anchor records the resolved tuple — the requested revision
      // for a live source (the snapshot pinned), the terminal's own
      // revision for a retirement-resolved source (contract §Repeat and
      // idempotency).
      const anchorRevision =
        terminal.value.source_entry_id === sourceEntryId
          ? sourceRevision
          : terminal.value.source_revision;
      const copy = copyForDestination(
        state,
        terminalEntry,
        terminal.value,
        destinationId,
        anchorRevision,
      );
      state.destinations.set(destinationId, copy);
      const response: KnowledgeAdoptResponse = {
        outcome: "adopted",
        entry: copy,
      };
      return spokeOk(response);
    },
  };
}

/**
 * The fixture scoped store over the shared destination map: the
 * `RemoteKnowledgeFace` backing store. `put` applies the shared
 * `assertAdoptionAnchorPreserved` gate as the provider-port atomic
 * backstop (the contract's two-layer rule) — the serving responder's
 * own refusal order (OCC fence before anchor gate) is what the wire
 * vectors measure.
 */
function fixtureScopedStore(state: FixtureState) {
  return {
    async getScopedKnowledgeEntry(
      _scope: Scope,
      entryId: string,
    ): Promise<SpokeResult<KnowledgeEntry>> {
      const entry = state.destinations.get(entryId);
      return entry === undefined
        ? spokeReject("KNOWLEDGE_ENTRY_NOT_FOUND", `no such entry ${entryId}`)
        : spokeOk(entry);
    },
    async putScopedKnowledgeEntry(
      _scope: Scope,
      entry: KnowledgeEntry,
      expectedBaseRevision: number | null,
    ): Promise<SpokeResult<KnowledgeEntry>> {
      const stored = state.destinations.get(entry.entry_id);
      const gate = assertAdoptionAnchorPreserved(entry, stored);
      if (!gate.ok) {
        return gate;
      }
      if (expectedBaseRevision === null) {
        if (stored !== undefined) {
          return spokeReject(
            "REVISION_CONFLICT",
            `Entry already exists: ${entry.entry_id}`,
            { entry_id: entry.entry_id },
          );
        }
      } else {
        if (stored === undefined) {
          return spokeReject(
            "STORED_REVISION_STALE",
            `KnowledgeEntry not found for update: ${entry.entry_id}`,
          );
        }
        const current = stored.revision ?? 0;
        if (current !== expectedBaseRevision) {
          return spokeReject(
            "STORED_REVISION_STALE",
            `Store revision ${current} does not match expected base ${expectedBaseRevision}`,
          );
        }
      }
      state.destinations.set(entry.entry_id, entry);
      return spokeOk(entry);
    },
    async listScopedKnowledgeEntries(
      _scope: Scope,
    ): Promise<SpokeResult<KnowledgeEntry[]>> {
      return spokeOk([...state.destinations.values()]);
    },
  };
}

// ── Loopback dial ─────────────────────────────────────────────────────────

interface DialedAdoptionHost {
  client: RemoteAdapter;
  close: () => void;
}

async function dialAdoptionHost(options: {
  ports: RemoteServePorts;
  clientCapabilities: string[];
  hostCapabilities?: string[];
}): Promise<DialedAdoptionHost> {
  const hostSeed = seed(0xa0);
  const hostPubkey = getPublicKeyEd25519(hostSeed);
  const hostPeerId = derivePeerIdFromEd25519Pubkey(hostPubkey);
  const clientSeed = seed(0xb0);
  const clientPubkey = getPublicKeyEd25519(clientSeed);
  const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
  const pair = loopbackTransportPair();
  const host = await connectResponder({
    transport: pair.server,
    identity: { seed: hostSeed },
    manifest: manifest(
      "adoption-conformance-host",
      options.hostCapabilities ?? HOST_CAPABILITIES,
    ),
    allowlist: [clientPeerId],
    peerKeys: { [clientPeerId]: clientPubkey },
    ports: options.ports,
  });
  const client = await connectRemoteAdapter({
    transport: pair.client,
    localIdentity: { seed: clientSeed },
    localManifest: manifest(
      "adoption-conformance-client",
      options.clientCapabilities,
    ),
    remotePubkey: hostPubkey,
    allowlist: [hostPeerId],
  });
  return {
    client,
    close: (): void => {
      client.close();
      host.close();
    },
  };
}

// ── Step execution ────────────────────────────────────────────────────────

type StepOutcome =
  | { kind: "value"; value: unknown }
  | { kind: "reject"; code: string; details: unknown };

interface StepRequest {
  op: string;
  request: JsonObject;
}

function parseStep(step: JsonObject, defaultOp?: string): StepRequest {
  const op = (step.op as string | undefined) ?? defaultOp;
  if (op === undefined) {
    throw new Error("step op");
  }
  return { op, request: (step.request ?? {}) as JsonObject };
}

/**
 * Dispatch a step. For `wire` the typed client is the boundary (adopt
 * and the knowledge ops); for `service` the raw request value goes
 * through `AdoptionPortService.invoke` verbatim.
 */
async function runStep(
  surface: string,
  host: DialedAdoptionHost | undefined,
  service: RemoteAdoptionService,
  clientPeer: string,
  step: StepRequest,
): Promise<StepOutcome> {
  if (surface === "service") {
    if (!step.op.startsWith("knowledge.")) {
      throw new Error(`service surface serves adoption ops only, got ${step.op}`);
    }
    const result = await service.invoke(clientPeer, step.op, step.request);
    return result.ok
      ? { kind: "value", value: result.value }
      : { kind: "reject", code: result.code, details: result.details };
  }
  if (host === undefined) {
    throw new Error("wire step needs a dialed host");
  }
  const clientScope: Scope = { scope_id: DESTINATION_SCOPE, extensions: {} };
  switch (step.op) {
    case "knowledge.adopt": {
      const result = await host.client.adoptKnowledgeEntry(
        PRINCIPAL,
        clientScope,
        step.request as unknown as KnowledgeAdoptRequest,
      );
      return result.ok
        ? { kind: "value", value: result.value }
        : { kind: "reject", code: result.code, details: result.details };
    }
    case "port.knowledge.get": {
      const result = await host.client.getKnowledgeEntry(
        step.request.entry_id as string,
      );
      return result.ok
        ? { kind: "value", value: result.value }
        : { kind: "reject", code: result.code, details: result.details };
    }
    case "port.knowledge.put": {
      const result = await host.client.putKnowledgeEntry(
        step.request.entry as KnowledgeEntry,
        (step.request.expected_base_revision ?? null) as number | null,
      );
      return result.ok
        ? { kind: "value", value: result.value }
        : { kind: "reject", code: result.code, details: result.details };
    }
    default:
      throw new Error(`unknown adoption op ${step.op}`);
  }
}

// ── Expect vocabulary ─────────────────────────────────────────────────────

/** `dotted.path[0]` lookup into a plain value. */
function pathGet(value: unknown, path: string): unknown {
  let current = value;
  for (const segment of path.split(".")) {
    const bracket = segment.indexOf("[");
    if (bracket >= 0) {
      const name = segment.slice(0, bracket);
      const index = Number(segment.slice(bracket + 1, -1));
      current =
        (current as JsonObject | undefined)?.[name] === undefined
          ? undefined
          : (current as JsonObject)[name];
      current = Array.isArray(current) ? current[index] : undefined;
    } else {
      current =
        current === null || typeof current !== "object"
          ? undefined
          : (current as JsonObject)[segment];
    }
  }
  return current;
}

/** Containment: every (nested) member of `expected` equals `actual`'s. */
function detailsContain(actual: unknown, expected: unknown): boolean {
  if (
    expected !== null &&
    typeof expected === "object" &&
    !Array.isArray(expected)
  ) {
    if (actual === null || typeof actual !== "object" || Array.isArray(actual)) {
      return false;
    }
    return Object.entries(expected as JsonObject).every(([key, want]) =>
      detailsContain((actual as JsonObject)[key], want),
    );
  }
  return JSON.stringify(actual) === JSON.stringify(expected);
}

function checkExpect(
  caseId: string,
  expectObj: JsonObject,
  outcome: StepOutcome,
  providerCalls: number,
): void {
  const code = expectObj.error_code as string | undefined;
  if (code !== undefined) {
    expect(
      outcome.kind,
      `case ${caseId}: expected reject ${code}, got ${JSON.stringify(outcome)}`,
    ).toBe("reject");
    if (outcome.kind !== "reject") {
      return;
    }
    if (code === "capability_missing" || code === "op_unsupported") {
      // Dispatch-layer wire codes arrive client-side mapped to
      // CAPABILITY_PORT_MISSING with details.wire_code preserved.
      expect(outcome.code, `case ${caseId}: mapped wire code`).toBe(
        "CAPABILITY_PORT_MISSING",
      );
      expect(
        pathGet(outcome.details, "wire_code"),
        `case ${caseId}: details.wire_code`,
      ).toBe(code);
    } else {
      expect(outcome.code, `case ${caseId}: reject code`).toBe(code);
    }
    if (expectObj.details !== undefined) {
      expect(
        detailsContain(outcome.details, expectObj.details),
        `case ${caseId}: details ${JSON.stringify(expectObj.details)} not contained in ${JSON.stringify(outcome.details)}`,
      ).toBe(true);
    }
    for (const key of (expectObj.absent_details ?? []) as string[]) {
      const present =
        outcome.details !== null &&
        typeof outcome.details === "object" &&
        key in (outcome.details as JsonObject);
      expect(
        present,
        `case ${caseId}: details.${key} must be absent (generic absence)`,
      ).toBe(false);
    }
  } else {
    expect(
      expectObj.success,
      `case ${caseId}: expect must carry error_code or success`,
    ).toBe(true);
    expect(
      outcome.kind,
      `case ${caseId}: expected success, got ${JSON.stringify(outcome)}`,
    ).toBe("value");
    if (outcome.kind !== "value") {
      return;
    }
    for (const assertion of (expectObj.state ?? []) as Array<{
      path: string;
      equals: unknown;
    }>) {
      expect(
        pathGet(outcome.value, assertion.path),
        `case ${caseId}: state.${assertion.path}`,
      ).toEqual(assertion.equals);
    }
    for (const path of (expectObj.present ?? []) as string[]) {
      expect(
        pathGet(outcome.value, path) !== undefined,
        `case ${caseId}: member ${path} must be present`,
      ).toBe(true);
    }
    for (const path of (expectObj.absent ?? []) as string[]) {
      expect(
        pathGet(outcome.value, path) === undefined,
        `case ${caseId}: member ${path} must be absent`,
      ).toBe(true);
    }
  }
  const wantCalls = expectObj.provider_calls as number | undefined;
  if (wantCalls !== undefined) {
    expect(providerCalls, `case ${caseId}: provider call count`).toBe(wantCalls);
  }
}

// ── Corpus ────────────────────────────────────────────────────────────────

interface CorpusCase {
  id: string;
  invariant: string;
  surface: string;
  outcome: string;
  what: string;
  input: CaseInput;
  setup?: Array<JsonObject>;
  op: string;
  request: JsonObject;
  expect: JsonObject;
  then?: Array<JsonObject>;
  requires?: string[];
}

interface Scenario {
  id: string;
  cases: CorpusCase[];
}

function loadScenarios(): Scenario[] {
  const corpusDir = new URL(
    "../../../../fixtures/adoption-conformance/",
    import.meta.url,
  );
  const files = readdirSync(corpusDir)
    .filter((name) => name.endsWith(".json"))
    .sort();
  return files.map((name) => {
    const doc = JSON.parse(
      readFileSync(new URL(name, corpusDir), "utf8"),
    ) as Scenario;
    return { id: doc.id, cases: doc.cases ?? [] };
  });
}

const REQUIRED_SCENARIOS = [
  "adopt-basic",
  "anchor-immutable",
  "idempotent-repeat",
  "private-source",
  "retirement-chain",
  "revision-conflict",
];

const EXPECT_KEYS: Record<string, true> = {
  error_code: true,
  details: true,
  absent_details: true,
  success: true,
  state: true,
  present: true,
  absent: true,
  provider_calls: true,
};

const CORPUS_OPS: Record<string, true> = {
  "knowledge.adopt": true,
  "port.knowledge.get": true,
  "port.knowledge.put": true,
};

describe("adoption-conformance corpus integrity", () => {
  it("covers the required scenarios with well-formed cases", () => {
    const scenarios = loadScenarios();
    const ids = scenarios.map((scenario) => scenario.id);
    for (const required of REQUIRED_SCENARIOS) {
      expect(ids, `scenario ${required} missing`).toContain(required);
    }
    const seen = new Set<string>();
    for (const scenario of scenarios) {
      for (const corpusCase of scenario.cases) {
        expect(seen.has(corpusCase.id), `duplicate case id ${corpusCase.id}`).toBe(
          false,
        );
        seen.add(corpusCase.id);
        expect(["wire", "service"]).toContain(corpusCase.surface);
        expect(corpusCase.outcome).toBe("identical");
        expect(typeof corpusCase.invariant).toBe("string");
        expect(typeof corpusCase.what).toBe("string");
        expect(typeof corpusCase.request).toBe("object");
        expect(CORPUS_OPS[corpusCase.op], `case ${corpusCase.id} op`).toBe(
          true,
        );
        for (const key of Object.keys(corpusCase.expect)) {
          expect(
            EXPECT_KEYS[key],
            `case ${corpusCase.id}: unknown expect key ${key}`,
          ).toBe(true);
        }
        expect(
          corpusCase.expect.error_code !== undefined ||
            corpusCase.expect.success !== undefined,
          `case ${corpusCase.id}: expect needs error_code or success`,
        ).toBe(true);
        for (const stepList of [corpusCase.setup ?? [], corpusCase.then ?? []]) {
          for (const step of stepList) {
            expect(
              CORPUS_OPS[step.op as string],
              `case ${corpusCase.id} step op`,
            ).toBe(true);
            expect(typeof step.request).toBe("object");
            for (const key of Object.keys(
              (step.expect ?? {}) as JsonObject,
            )) {
              expect(
                EXPECT_KEYS[key],
                `case ${corpusCase.id}: unknown step expect key ${key}`,
              ).toBe(true);
            }
          }
        }
        for (const source of corpusCase.input.sources ?? []) {
          expect(typeof source.entry_id).toBe("string");
          expect(typeof source.entry).toBe("object");
          expect(source.entry.entry_id).toBe(source.entry_id);
        }
      }
    }
  });
});

/**
 * Run one case end-to-end: seed the fixture tables, attach the service
 * (or not), dial the loopback for wire cases, execute setup → op → then,
 * and assert every step's expect vocabulary.
 */
async function runCase(corpusCase: CorpusCase): Promise<void> {
  const surface = corpusCase.surface ?? "wire";
  const input = corpusCase.input ?? {};
  const state = fixtureState(input);
  const port = scriptedAdoptionPort(state);
  const viewpoint = input.admitted_viewpoint;
  const service: RemoteAdoptionService = new AdoptionPortService(
    port,
    (peerId) =>
      spokeOk({
        principal: { principal_id: `prn_${peerId}`, extensions: {} },
        scope: {
          scope_id: DESTINATION_SCOPE,
          ...(viewpoint !== undefined ? { viewpoint } : {}),
          extensions: {},
        },
      }),
    input.dedup_capacity === undefined
      ? undefined
      : {
          dedupStore: new InMemoryHistoryDedupStore({
            maxEntries: input.dedup_capacity,
          }),
        },
  );
  const adoptionFace = input.adoption_face ?? true;
  const clientCaps = input.client_capabilities ?? CLIENT_CAPABILITIES;
  const hostCaps = input.host_capabilities ?? HOST_CAPABILITIES;

  // Wire surface: dial a responder whose composite carries the service
  // (or not) plus the knowledge face over the shared destination store;
  // service surface talks to the service directly.
  const host =
    surface === "wire"
      ? await dialAdoptionHost({
          clientCapabilities: clientCaps,
          hostCapabilities: hostCaps,
          ports: {
            knowledge: {
              store: fixtureScopedStore(state),
              admission: () =>
                spokeOk({ scope_id: DESTINATION_SCOPE, extensions: {} }),
            },
            asAdoption: adoptionFace ? () => service : undefined,
          },
        })
      : undefined;
  const clientPeer = derivePeerIdFromEd25519Pubkey(
    getPublicKeyEd25519(seed(0xb0)),
  );

  try {
    const execute = async (step: StepRequest): Promise<StepOutcome> =>
      runStep(surface, host, service, clientPeer, step);

    // setup steps: execute, honoring an optional per-step expect.
    for (const step of corpusCase.setup ?? []) {
      const parsed = parseStep(step);
      const outcome = await execute(parsed);
      if (step.expect !== undefined) {
        checkExpect(corpusCase.id, step.expect as JsonObject, outcome, state.calls.length);
      } else if (outcome.kind === "reject") {
        throw new Error(
          `setup step rejected: ${JSON.stringify(outcome)}`,
        );
      }
    }

    // The op under test.
    const parsedOp = parseStep(corpusCase as unknown as JsonObject);
    const outcome = await execute(parsedOp);
    checkExpect(corpusCase.id, corpusCase.expect, outcome, state.calls.length);

    // then steps.
    for (const step of corpusCase.then ?? []) {
      const parsed = parseStep(step);
      const outcome = await execute(parsed);
      if (step.expect !== undefined) {
        checkExpect(corpusCase.id, step.expect as JsonObject, outcome, state.calls.length);
      }
    }
  } finally {
    host?.close();
  }
}

describe("adoption wire conformance (fixtures/adoption-conformance)", () => {
  const scenarios = loadScenarios();
  for (const scenario of scenarios) {
    for (const corpusCase of scenario.cases) {
      it(`${scenario.id}/${corpusCase.id}`, async () => {
        await runCase(corpusCase);
      });
    }
  }
});

// ── Dedup store invariants (B3 single-flight, recorded-outcome snapshots) ──

const DEDUP_ADMISSION = (peerId: string) =>
  spokeOk({
    principal: { principal_id: `prn_${peerId}`, extensions: {} },
    scope: { scope_id: DESTINATION_SCOPE, extensions: {} },
  });

function adoptPayload(idempotencyKey: string): JsonObject {
  return {
    source_scope_id: SOURCE_SCOPE,
    source_entry_id: "ke_shared",
    source_revision: 1,
    destination_scope_id: DESTINATION_SCOPE,
    idempotency_key: idempotencyKey,
    extensions: {},
  };
}

function adoptedResponse(): KnowledgeAdoptResponse & JsonObject {
  return {
    outcome: "adopted",
    entry: {
      schema_version: 1,
      entry_id: "ke_shared",
      entry_type: "note",
      canonical_name: "shared",
      status: "confirmed",
      revision: 1,
      body: {},
      extensions: {},
      adoption_anchor: {
        schema_version: 1,
        source_scope_id: SOURCE_SCOPE,
        source_entry_id: "ke_shared",
        source_revision: 1,
        extensions: {},
      },
    },
  } as KnowledgeAdoptResponse & JsonObject;
}

describe("adoption dedup single-flight (contract §Repeat and idempotency)", () => {
  it("one store shared by two services yields at most one in-flight invocation per retry identity; unrelated keys stay independent", async () => {
    const shared = new InMemoryHistoryDedupStore();
    const calls: string[] = [];
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const makePort = (): AdoptionPort => ({
      adoptKnowledgeEntry: async () => {
        calls.push("call");
        // only the first invocation is held — the unrelated key must
        // finish while the held call is still in flight.
        if (calls.length === 1) {
          await gate;
        }
        return spokeOk(adoptedResponse());
      },
    });
    const serviceA = new AdoptionPortService(makePort(), DEDUP_ADMISSION, {
      dedupStore: shared,
    });
    const serviceB = new AdoptionPortService(makePort(), DEDUP_ADMISSION, {
      dedupStore: shared,
    });
    const first = serviceA.invoke("peer_a", "knowledge.adopt", adoptPayload("k1"));
    const second = serviceB.invoke("peer_a", "knowledge.adopt", adoptPayload("k1"));
    // The same key on a second service sharing the store waits; an
    // unrelated key never blocks behind the held invocation.
    const third = await serviceA.invoke(
      "peer_a",
      "knowledge.adopt",
      adoptPayload("k2"),
    );
    expect(third.ok).toBe(true);
    // the held k1 call plus the independent k2 call — no third
    // invocation for the still-in-flight k1 replay.
    expect(calls.length).toBe(2);
    release();
    const [firstResult, secondResult] = await Promise.all([first, second]);
    expect(firstResult.ok).toBe(true);
    expect(secondResult.ok).toBe(true);
    // note: at most one provider invocation per retry identity across
    // both services; the replay returns the recorded outcome.
    expect(calls.length).toBe(2);
  });

  it("a provider invocation that throws releases the single-flight claim — the next same-key call invokes again", async () => {
    let calls = 0;
    const port: AdoptionPort = {
      adoptKnowledgeEntry: async () => {
        calls += 1;
        if (calls === 1) {
          throw new Error("provider crashed before settling");
        }
        return spokeOk(adoptedResponse());
      },
    };
    const service = new AdoptionPortService(port, DEDUP_ADMISSION, {
      dedupStore: new InMemoryHistoryDedupStore(),
    });
    await expect(
      service.invoke("peer_a", "knowledge.adopt", adoptPayload("k3")),
    ).rejects.toThrow("provider crashed");
    // note: the unsettled invocation released its claim — no stale
    // recorded outcome, and the retry reaches the provider again (B3:
    // recorded-outcome dedup covers settled outcomes only).
    const retry = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      adoptPayload("k3"),
    );
    expect(retry.ok).toBe(true);
    expect(calls).toBe(2);
  });

  it("a recorded provider refusal is snapshotted — mutating the provider's object never changes the replay", async () => {
    const mutableDetails: JsonObject = { nested: { count: 1 } };
    const port: AdoptionPort = {
      adoptKnowledgeEntry: async () =>
        spokeReject("REVISION_CONFLICT", "conflict", {
          nested: mutableDetails.nested as Record<string, unknown>,
        }),
    };
    const service = new AdoptionPortService(port, DEDUP_ADMISSION, {
      dedupStore: new InMemoryHistoryDedupStore(),
    });
    const first = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      adoptPayload("k4"),
    );
    expect(first.ok).toBe(false);
    (mutableDetails.nested as Record<string, unknown>).count = 999;
    const replay = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      adoptPayload("k4"),
    );
    expect(replay.ok).toBe(false);
    if (!replay.ok) {
      expect(pathGet(replay.details, "nested.count")).toBe(1);
    }
  });
});

describe("adoption dedup single-flight fallback (lookup/record-only store — F-007)", () => {
  /** A store with no `settle` — the serving layer's shared keyed claim runs. */
  const lookupOnlyStore = (): HistoryDedupStore => ({
    lookup: async () => ({ status: "unseen" }),
    record: async () => {},
  });

  it("the shared fallback serializes the same retry identity across services; an unrelated key proceeds", async () => {
    const shared = lookupOnlyStore();
    const callsA: string[] = [];
    const callsB: string[] = [];
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const portA: AdoptionPort = {
      adoptKnowledgeEntry: async () => {
        callsA.push("call");
        await gate;
        return spokeOk(adoptedResponse());
      },
    };
    const portB: AdoptionPort = {
      adoptKnowledgeEntry: async () => {
        callsB.push("call");
        return spokeOk(adoptedResponse());
      },
    };
    const serviceA = new AdoptionPortService(portA, DEDUP_ADMISSION, {
      dedupStore: shared,
    });
    const serviceB = new AdoptionPortService(portB, DEDUP_ADMISSION, {
      dedupStore: shared,
    });
    const first = serviceA.invoke("peer_a", "knowledge.adopt", adoptPayload("k-f1"));
    const second = serviceB.invoke("peer_a", "knowledge.adopt", adoptPayload("k-f1"));
    // An unrelated retry identity on B proceeds while A's same-key call
    // is held — independent keys never queue behind another identity.
    const third = await serviceB.invoke(
      "peer_a",
      "knowledge.adopt",
      adoptPayload("k-f2"),
    );
    expect(third.ok).toBe(true);
    expect(callsA).toHaveLength(1);
    expect(callsB).toHaveLength(1);
    release();
    const [firstResult, secondResult] = await Promise.all([first, second]);
    expect(firstResult.ok).toBe(true);
    expect(secondResult.ok).toBe(true);
    // The held same-key call serialized behind the fallback claim and
    // invoked its provider AFTER the first call settled — the
    // lookup/record-only store reports Unseen both times, so both runs
    // execute, but never in parallel.
    expect(callsA).toHaveLength(1); // only the first same-key call used A's port
    expect(callsB).toHaveLength(2); // the unrelated key + the released same-key call
  });

  it("an unsettled invocation releases the fallback claim — the retry invokes again", async () => {
    let calls = 0;
    const port: AdoptionPort = {
      adoptKnowledgeEntry: async () => {
        calls += 1;
        if (calls === 1) {
          throw new Error("provider crashed before settling");
        }
        return spokeOk(adoptedResponse());
      },
    };
    const service = new AdoptionPortService(port, DEDUP_ADMISSION, {
      dedupStore: lookupOnlyStore(),
    });
    await expect(
      service.invoke("peer_a", "knowledge.adopt", adoptPayload("k-f3")),
    ).rejects.toThrow("provider crashed");
    const retry = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      adoptPayload("k-f3"),
    );
    expect(retry.ok).toBe(true);
    expect(calls).toBe(2);
  });

  it("admits the inclusive 2^53 endpoint for source_revision and inside a request extension (F-013/F-006)", async () => {
    // note: `9007199254740993` parses to exactly `9007199254740992`
    // (2^53) — the same wire number Rust's adoption path admits; the
    // payload guard, the request decoder and the anchor deep-check all
    // share the inclusive bound.
    const calls: number[] = [];
    const port: AdoptionPort = {
      adoptKnowledgeEntry: async (_p, _s, request) => {
        calls.push(request.source_revision);
        return spokeOk(adoptedResponse());
      },
    };
    const service = new AdoptionPortService(port, DEDUP_ADMISSION, {
      dedupStore: new InMemoryHistoryDedupStore(),
    });
    for (const [key, revision] of [
      ["k-edge", 9007199254740992],
      ["k-edge-2", 9007199254740993], // parses to exactly 2^53
    ] as const) {
      const result = await service.invoke("peer_a", "knowledge.adopt", {
        ...adoptPayload(key),
        source_revision: revision,
      });
      expect(result.ok, `source_revision ${revision} must be admitted`).toBe(
        true,
      );
    }
    expect(calls).toEqual([9007199254740992, 9007199254740992]);
    // The same endpoint inside a request extension passes the payload
    // guard too.
    const extensionEndpoint = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      {
        ...adoptPayload("k-edge-3"),
        extensions: { product: { v: 9007199254740992 } },
      },
    );
    expect(extensionEndpoint.ok).toBe(true);
  });
});

describe("served result E1 revision domain + E2 extensions presence", () => {
  const servedEntry = (anchorRevision: number, entryRevision: number) => ({
    schema_version: 1,
    entry_id: "ke_dst",
    entry_type: "note",
    canonical_name: "copy",
    status: "confirmed",
    revision: entryRevision,
    body: {},
    adoption_anchor: {
      schema_version: 1,
      source_scope_id: "spc_local",
      source_entry_id: "ke_src",
      source_revision: anchorRevision,
      extensions: {},
    },
    extensions: {},
  });
  const serveWith = (result: unknown) =>
    new AdoptionPortService(
      {
        adoptKnowledgeEntry: async () =>
          spokeOk(result as KnowledgeAdoptResponse),
      },
      DEDUP_ADMISSION,
      { dedupStore: new InMemoryHistoryDedupStore() },
    );
  const localPayload = () => ({
    ...adoptPayload("k-e1"),
    source_scope_id: DESTINATION_SCOPE,
  });

  it("refuses a served anchor or entry revision outside [0, 2^53] — the inclusive endpoint serves (E1)", async () => {
    for (const [anchorRevision, entryRevision, servedOk] of [
      [9007199254740992, 1, true],
      [9007199254740994, 1, false],
      [1, 9007199254740992, true],
      [1, 9007199254740994, false],
    ] as const) {
      const service = serveWith({
        outcome: "adopted",
        entry: servedEntry(anchorRevision, entryRevision),
      });
      const result = await service.invoke(
        "peer_a",
        "knowledge.adopt",
        localPayload(),
      );
      expect(result.ok, `anchor=${anchorRevision} entry=${entryRevision}`).toBe(
        servedOk,
      );
    }
  });

  it("a producer-supplied response extensions member survives verbatim (E2)", async () => {
    const service = serveWith({
      outcome: "adopted",
      entry: servedEntry(3, 1),
      extensions: { product: { v: 1 } },
    });
    const result = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      localPayload(),
    );
    expect(result.ok).toBe(true);
    if (result.ok && result.value !== null && typeof result.value === "object") {
      // note: `extensions` is an optional declared response member —
      // the served value preserves the producer's member verbatim.
      expect("extensions" in result.value && result.value.extensions).toEqual(
        { product: { v: 1 } },
      );
    }
  });
});

describe("served result failure branch + extension validation (E3 / qc3 F-021/F-023)", () => {
  const localPayload = () => ({
    ...adoptPayload("k-e1"),
    source_scope_id: DESTINATION_SCOPE,
  });
  const serveWith = (result: unknown) =>
    new AdoptionPortService(
      {
        adoptKnowledgeEntry: async () =>
          spokeOk(result as KnowledgeAdoptResponse),
      },
      DEDUP_ADMISSION,
      { dedupStore: new InMemoryHistoryDedupStore() },
    );

  it("a failure branch with a supplied extensions member serves verbatim (E3 + E2)", async () => {
    const service = serveWith({
      error: {
        code: "REVISION_CONFLICT",
        message: "conflict",
        details: {},
        extensions: {},
      },
      extensions: { product: { v: 1 } },
    });
    const result = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      localPayload(),
    );
    expect(result.ok).toBe(true);
    if (result.ok && result.value !== null && typeof result.value === "object") {
      expect(result.value).toMatchObject({
        error: { code: "REVISION_CONFLICT", message: "conflict" },
        extensions: { product: { v: 1 } },
      });
    }
  });

  it("a malformed response extensions member refuses INTERNAL_ERROR on both branches (qc3 F-023)", async () => {
    for (const branch of [
      { outcome: "adopted", entry: { entry_id: "ke_dst" } },
      {
        error: {
          code: "REVISION_CONFLICT",
          message: "conflict",
          details: {},
          extensions: {},
        },
      },
    ]) {
      for (const extensions of [{ "Bad-Key": {} }, { ns: 5 }]) {
        const service = serveWith({ ...branch, extensions });
        const result = await service.invoke(
          "peer_a",
          "knowledge.adopt",
          localPayload(),
        );
        expect(result.ok).toBe(false);
        if (!result.ok) {
          expect(result.code).toBe("INTERNAL_ERROR");
        }
      }
    }
  });
});

// ── Destination admission at the serving boundary (C7 / QC2-08) ────────

describe("adoption destination admission (contract Wire-wide rules)", () => {
  it("a destination scope outside the admitted context answers NOT_AUTHORIZED with zero provider calls — before dedup, single-flight and source work", async () => {
    const calls: string[] = [];
    const port: AdoptionPort = {
      adoptKnowledgeEntry: async () => {
        calls.push("call");
        return spokeOk(adoptedResponse());
      },
    };
    const service = new AdoptionPortService(port, DEDUP_ADMISSION);
    // The admission binds `spc_local`; a request naming another
    // destination space_id refuses before the provider runs.
    const denied = await service.invoke("peer_a", "knowledge.adopt", {
      ...adoptPayload("dst-1"),
      destination_scope_id: "spc_elsewhere",
    });
    expect(denied.ok).toBe(false);
    if (!denied.ok) {
      expect(denied.code).toBe("NOT_AUTHORIZED");
    }
    expect(calls).toHaveLength(0);

    // A same-key replay of the admitted destination still works.
    const allowed = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      adoptPayload("dst-2"),
    );
    expect(allowed.ok).toBe(true);
    expect(calls).toHaveLength(1);
  });
});

describe("adoption request extensions and numeric parity", () => {
  it("an extension float member that Rust parses as f64 digests identically on both stacks (extension-number parity)", async () => {
    // note: qc1 F-013 — `1e20` in an extension value arrives at Rust as
    // f64 and passes its integer-identity gate; TS must accept it the
    // same way and refuse only an integer-literal member beyond the
    // JCS exact-integer range.
    const port: AdoptionPort = {
      adoptKnowledgeEntry: async () => spokeOk(adoptedResponse()),
    };
    const service = new AdoptionPortService(port, DEDUP_ADMISSION, {
      dedupStore: new InMemoryHistoryDedupStore(),
    });
    const withFloat = await service.invoke("peer_a", "knowledge.adopt", {
      ...adoptPayload("k5"),
      extensions: { product: { magnitude: 1e20 } },
    });
    expect(withFloat.ok).toBe(true);
    const badKey = await service.invoke("peer_a", "knowledge.adopt", {
      ...adoptPayload("k6"),
      extensions: { "Bad-Namespace": { v: 1 } },
    });
    expect(badKey.ok).toBe(false);
    if (!badKey.ok) {
      expect(badKey.code).toBe("INVALID_INPUT");
    }
    const scalarValue = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      { ...adoptPayload("k7"), extensions: { product: 5 } },
    );
    expect(scalarValue.ok).toBe(false);
    if (!scalarValue.ok) {
      expect(scalarValue.code).toBe("INVALID_INPUT");
    }
    // note: qc1 F-013 / qc3 F-006 — the inclusive `2^53` endpoint: JS
    // parses `9007199254740993` to exactly `9007199254740992`, the same
    // wire number Rust's adoption path admits; only an integral member
    // strictly beyond `2^53` (and below `2^64`) refuses.
    const endpoint = await service.invoke("peer_a", "knowledge.adopt", {
      ...adoptPayload("k8"),
      extensions: { product: { v: 9007199254740992 } },
    });
    expect(endpoint.ok).toBe(true);
    const tooBigInteger = await service.invoke(
      "peer_a",
      "knowledge.adopt",
      { ...adoptPayload("k9"), extensions: { product: { v: 1e16 } } },
    );
    expect(tooBigInteger.ok).toBe(false);
    if (!tooBigInteger.ok) {
      expect(tooBigInteger.code).toBe("INVALID_INPUT");
    }
  });

// ── Scoped put caller-OCC fence (C1 / F-001 / QC2-01) ───────────────────

describe("scoped put caller OCC fence (spoke-knowledge-adoption wire-wide C1)", () => {
  /**
   * A CAS-only scoped store: enforces the OCC fence it is handed but
   * applies no anchor gate of its own — the responder's pre-read plus
   * caller fence is the whole guarantee under test.
   */
  function casOnlyStore(state: FixtureState) {
    const seenBases: Array<number | null> = [];
    const store = {
      async getScopedKnowledgeEntry(
        _scope: Scope,
        entryId: string,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        const entry = state.destinations.get(entryId);
        return entry === undefined
          ? spokeReject("KNOWLEDGE_ENTRY_NOT_FOUND", `no such entry ${entryId}`)
          : spokeOk(entry);
      },
      async putScopedKnowledgeEntry(
        _scope: Scope,
        entry: KnowledgeEntry,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        seenBases.push(expectedBaseRevision);
        const stored = state.destinations.get(entry.entry_id);
        if (expectedBaseRevision === null) {
          if (stored !== undefined) {
            return spokeReject("REVISION_CONFLICT", "exists");
          }
        } else if (stored === undefined || (stored.revision ?? 0) !== expectedBaseRevision) {
          return spokeReject("STORED_REVISION_STALE", "stale");
        }
        state.destinations.set(entry.entry_id, entry);
        return spokeOk(entry);
      },
      async listScopedKnowledgeEntries(
        _scope: Scope,
      ): Promise<SpokeResult<KnowledgeEntry[]>> {
        return spokeOk([...state.destinations.values()]);
      },
    };
    return { store, seenBases };
  }

  function anchoredEntry(entryId: string, revision: number): KnowledgeEntry {
    return {
      schema_version: 1,
      entry_id: entryId,
      entry_type: "note",
      canonical_name: entryId,
      status: "confirmed",
      revision,
      body: {},
      extensions: {},
      adoption_anchor: {
        schema_version: 1,
        source_scope_id: SOURCE_SCOPE,
        source_entry_id: "ke_source",
        source_revision: 3,
        extensions: {},
      },
    } as KnowledgeEntry;
  }

  async function dialPutHost(state: FixtureState): Promise<DialedAdoptionHost> {
    const { store } = casOnlyStore(state);
    return dialAdoptionHost({
      clientCapabilities: CLIENT_CAPABILITIES,
      ports: {
        knowledge: {
          store,
          admission: () =>
            spokeOk({ scope_id: DESTINATION_SCOPE, extensions: {} }),
        },
      },
    });
  }

  it("a stale caller base refuses even when the anchor is unchanged; the matching base writes through the store", async () => {
    const state = fixtureState({
      destinations: [anchoredEntry("ke_dest", 5)],
    });
    const host = await dialPutHost(state);
    try {
      // The caller claims base 6 but the responder's snapshot is revision 5:
      // an anchor check on 5 can never justify a write whose CAS fence is 6.
      const stale = await host.client.putKnowledgeEntry(
        { ...anchoredEntry("ke_dest", 6), canonical_name: "stale" },
        6,
      );
      expect(stale.ok).toBe(false);
      if (!stale.ok) {
        expect(stale.code).toBe("STORED_REVISION_STALE");
      }
      expect(state.destinations.get("ke_dest")?.revision).toBe(5);

      // Update against a missing destination fails the caller fence too
      // (create-only null base is the only write to an absent entry).
      const missing = await host.client.putKnowledgeEntry(
        anchoredEntry("ke_absent", 1),
        0,
      );
      expect(missing.ok).toBe(false);
      if (!missing.ok) {
        expect(missing.code).toBe("STORED_REVISION_STALE");
      }

      // A null base against an occupied slot keeps the create-only refusal.
      const occupiedCreate = await host.client.putKnowledgeEntry(
        anchoredEntry("ke_dest", 1),
        null,
      );
      expect(occupiedCreate.ok).toBe(false);
      if (!occupiedCreate.ok) {
        expect(occupiedCreate.code).toBe("REVISION_CONFLICT");
      }

      // The correct caller base — the inspected snapshot's revision — is
      // forwarded verbatim as the CAS fence and writes.
      const correct = await host.client.putKnowledgeEntry(
        { ...anchoredEntry("ke_dest", 6), canonical_name: "written" },
        5,
      );
      expect(correct.ok).toBe(true);
      expect(state.destinations.get("ke_dest")?.canonical_name).toBe("written");
    } finally {
      host.close();
    }
  });
});
});
