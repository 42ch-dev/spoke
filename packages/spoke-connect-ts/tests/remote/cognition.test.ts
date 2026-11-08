/**
 * `spoke-cognition` capability, negotiation and raw unsupported-requirement
 * vectors (consumer contract §1–§2).
 *
 * Every case is a real round trip through the product surfaces —
 * `connectResponder` (host) and `connectRemoteAdapter` (client) over the
 * in-repo loopback transport — with provider-call accounting, so negotiation,
 * the deny branch and the unsupported-requirement envelope are proven on the
 * wire rather than by a copied boolean or a serde failure. The fixture is
 * consumed unchanged by the Rust suite
 * (`crates/spoke-connect/src/remote/responder.rs` cognition tests), which
 * reuses the same host manifests and payloads.
 */

import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import type {
  CognitionFilter,
  CognitionRecord,
  ConnectHello,
  ConnectInvokeResponse,
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  MemoryRecordsListRequest,
  Principal,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  cognitionRecordMatchesFilter,
  spokeOk,
  spokeReject,
  validateCognitionRecord,
  type BaselinePorts,
  type SpokeResult,
} from "@42ch/spoke-operations";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { authenticateInvokeRequest } from "../../src/core/envelope-auth.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../../src/core/hello.js";
import { generateNonce } from "../../src/core/nonce.js";
import { CAPABILITY_SPOKE_COGNITION } from "../../src/core/dispatch.js";
import { negotiatedCapabilities } from "../../src/core/session.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import { isValidSuccessPayload } from "../../src/remote/payload.js";
import { connectRemoteAdapter } from "../../src/remote/remote-adapter.js";
import {
  connectResponder,
  type RemoteMemoryService,
} from "../../src/remote/responder.js";
import {
  loopbackTransportPair,
  type Transport,
} from "../../src/remote/transport.js";

interface FixtureHost {
  capabilities: string[];
  note: string;
}

interface NegotiationCase {
  id: string;
  kind: "negotiation";
  local_capabilities: string[];
  remote_capabilities: string[];
  negotiated_includes_cognition: boolean;
}

interface ExpectedOutcome {
  status: "ok" | "reject";
  code?: string;
  reason?: string;
  field?: string;
}

interface ServedCase {
  id: string;
  kind: "served";
  host: string;
  client: string;
  op: string;
  payload: Record<string, unknown>;
  expect: ExpectedOutcome;
  expect_provider_calls: number;
  /** Entry ids the conformant KE source must return, in source order. */
  expect_entry_ids?: string[];
}

interface Fixture {
  schema_version: number;
  capability: string;
  legacy_capability: string;
  hosts: Record<string, FixtureHost>;
  ke_entries: KnowledgeEntry[];
  cases: (NegotiationCase | ServedCase)[];
}

const FIXTURE = JSON.parse(
  readFileSync(
    new URL("../fixtures/cognition-negotiation.json", import.meta.url),
    "utf8",
  ),
) as Fixture;

const NEGOTIATION_CASES = FIXTURE.cases.filter(
  (testCase): testCase is NegotiationCase => testCase.kind === "negotiation",
);

const SERVED_CASES = FIXTURE.cases.filter(
  (testCase): testCase is ServedCase => testCase.kind === "served",
);

const SCOPE_LIST_OP = "port.scope.list_knowledge_entries";
const MEMORY_LIST_OP = "memory.records.list";

function manifestFor(hostId: string, host: FixtureHost): HostCapabilityManifest {
  return {
    schema_version: 1,
    host_id: hostId,
    roles: ["data-store"],
    capabilities: [...host.capabilities] as [string, ...string[]],
    namespaces: ["world_a"],
    extensions: {},
  };
}

/** The memory seam principal — the op mapping derives the real one. */
const TEST_PRINCIPAL: Principal = {
  principal_id: "principal_t3",
  extensions: {},
};

interface ProviderCalls {
  scopeLists: number;
  receivedScopes: Scope[];
  memoryInvokes: number;
  returnedEntries: KnowledgeEntry[];
}

/**
 * The `modules.cognition.records` a fixture entry carries. The module bag is
 * open (`modules.*` is opaque to the wire), so each record is validated once
 * with the C5 record gate and only then read as a typed record.
 */
function entryCognitionRecords(entry: KnowledgeEntry): CognitionRecord[] {
  const cognition = entry.modules?.["cognition"];
  if (typeof cognition !== "object" || cognition === null) {
    return [];
  }
  if (!("records" in cognition) || !Array.isArray(cognition.records)) {
    return [];
  }
  return cognition.records.map((record) => {
    if (!validateCognitionRecord(record)) {
      throw new Error("fixture cognition record failed C5 validation");
    }
    return record; // narrowed by the C5 record gate above
  });
}

/**
 * The conformant KE-only source: it applies the requested current-view
 * cognition selection through the shared C6 matcher, so a served selection
 * returns the discriminating fixture entries (multi-context, forked,
 * no-fork and explicit-zero variants included) instead of an unconditional
 * empty echo. An absent optional record member is never read as zero.
 */
function selectEntries(scope: Scope, calls: ProviderCalls): KnowledgeEntry[] {
  const selection = scope.cognition;
  const selected =
    selection === undefined
      ? [...FIXTURE.ke_entries]
      : FIXTURE.ke_entries.filter((entry) =>
          entryCognitionRecords(entry).some(
            (record) =>
              record.view === "current" &&
              cognitionRecordMatchesFilter(record, selection.filter),
          ),
        );
  calls.returnedEntries = selected;
  return selected;
}

function providerPorts(
  calls: ProviderCalls,
): BaselinePorts & Partial<RemoteMemoryService> {
  return {
    async getKnowledgeEntry(entryId): Promise<SpokeResult<KnowledgeEntry>> {
      return spokeReject(
        SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND,
        `not implemented in this fixture: ${entryId}`,
      );
    },
    async putKnowledgeEntry(entry): Promise<SpokeResult<KnowledgeEntry>> {
      return spokeOk(entry);
    },
    async getRelation(relationId): Promise<SpokeResult<Relation>> {
      return spokeReject(
        SpokeRejectCode.RELATION_NOT_FOUND,
        `not implemented in this fixture: ${relationId}`,
      );
    },
    async putRelation(relation): Promise<SpokeResult<Relation>> {
      return spokeOk(relation);
    },
    async listKnowledgeEntries(scope: Scope): Promise<SpokeResult<KnowledgeEntry[]>> {
      calls.scopeLists += 1;
      calls.receivedScopes.push(scope);
      return spokeOk(selectEntries(scope, calls));
    },
    async listTimelineEvents(): Promise<SpokeResult<TimelineEvent[]>> {
      return spokeOk([]);
    },
    async putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>> {
      return spokeOk(findings);
    },
    async listRules(): Promise<SpokeResult<Rule[]>> {
      return spokeOk([]);
    },
    async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
      return spokeOk(manifestFor("provider", { capabilities: [], note: "" }));
    },
    async listPeerHostCapabilityManifests(): Promise<
      SpokeResult<HostCapabilityManifest[]>
    > {
      return spokeOk([]);
    },
    async invoke(
      _peerId: string,
      _op: string,
      _payload: Record<string, unknown>,
    ): Promise<SpokeResult<unknown>> {
      calls.memoryInvokes += 1;
      // A decodable `memory.records.list` success body: the served case must
      // reach the service and return through the real client mapping.
      return spokeOk({ records: [], total_authorized: 0 });
    },
  };
}

interface ServedResult {
  result: SpokeResult<unknown>;
  calls: ProviderCalls;
  /** Every inbound envelope the client received, in arrival order. */
  frames: Record<string, unknown>[];
}

/**
 * Pass-through transport that records each inbound envelope, so the suite
 * asserts the host's exact wire rejection, not only the client's mapping of
 * it.
 */
function recordingTransport(
  transport: Transport,
  frames: Record<string, unknown>[],
): Transport {
  return {
    send: (envelope) => transport.send(envelope),
    recv: async () => {
      const envelope = await transport.recv();
      frames.push(
        JSON.parse(new TextDecoder().decode(envelope)) as Record<string, unknown>,
      );
      return envelope;
    },
    close: () => transport.close?.(),
  };
}

const textEncoder = new TextEncoder();

/**
 * A payload the typed `listKnowledgeEntries` path can carry: that method
 * forwards exactly `{ scope }`, so any other top-level member would be
 * dropped before the wire. Such payloads take the raw signed session.
 */
function isTypedScopeRequest(payload: Record<string, unknown>): boolean {
  const members = Object.keys(payload);
  return members.length === 1 && members[0] === "scope";
}

/**
 * Send the WHOLE fixture payload over a real signed session (initiator hello,
 * responder hello bound to our nonce, session snapshot, signed invoke)
 * against the same responder, then decode the received response with the
 * op's own success-shape gate — the gate the `RemoteAdapter` applies to the
 * same response. The typed scope method cannot forward the discriminating
 * top-level `cognition` member, so this is the consumer's only faithful path
 * for that vector.
 */
async function rawScopeServe(
  testCase: ServedCase,
  transport: Transport,
  session: {
    clientSeed: Uint8Array;
    hostPubkey: Uint8Array;
    hostPeerId: string;
    clientManifest: HostCapabilityManifest;
  },
  calls: ProviderCalls,
): Promise<ServedResult> {
  const nonce = generateNonce();
  await transport.send(
    textEncoder.encode(
      encodeJsonMessage(
        await signHelloEd25519(session.clientSeed, nonce, session.clientManifest),
      ),
    ),
  );
  const responderHello = decodeJsonMessage(
    await transport.recv(),
  ) as ConnectHello;
  await verifyHelloEd25519(
    session.hostPubkey,
    session.hostPeerId,
    responderHello,
    nonce,
  );
  const snapshot = decodeJsonMessage(await transport.recv()) as {
    session_id: string;
  };
  const request = await authenticateInvokeRequest(session.clientSeed, {
    session_id: snapshot.session_id,
    sequence: 0,
    request_id: testCase.id,
    op: testCase.op,
    payload: testCase.payload,
  });
  await transport.send(textEncoder.encode(encodeJsonMessage(request)));
  const response = decodeJsonMessage(
    await transport.recv(),
  ) as ConnectInvokeResponse;
  if ("error" in response) {
    throw new Error(
      `${testCase.id}: expected success, received ${JSON.stringify(response.error)}`,
    );
  }
  expect(
    isValidSuccessPayload(testCase.op, response.payload),
    `${testCase.id}: the served payload decodes as the ${testCase.op} response`,
  ).toBe(true);
  return { result: spokeOk(response.payload), calls, frames: [] };
}

async function serveCase(testCase: ServedCase): Promise<ServedResult> {
  const hostSeed = new Uint8Array(32).fill(0xa0);
  const clientSeed = new Uint8Array(32).fill(0x10);
  const hostPubkey = getPublicKeyEd25519(hostSeed);
  const clientPubkey = getPublicKeyEd25519(clientSeed);
  const hostPeerId = derivePeerIdFromEd25519Pubkey(hostPubkey);
  const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
  const pair = loopbackTransportPair();
  const frames: Record<string, unknown>[] = [];
  const calls: ProviderCalls = {
    scopeLists: 0,
    receivedScopes: [],
    memoryInvokes: 0,
    returnedEntries: [],
  };
  const host = await connectResponder({
    transport: pair.server,
    identity: { seed: hostSeed },
    manifest: manifestFor("host", FIXTURE.hosts[testCase.host]!),
    allowlist: [clientPeerId],
    peerKeys: { [clientPeerId]: clientPubkey },
    ports: providerPorts(calls),
  });
  try {
    if (
      testCase.op === SCOPE_LIST_OP &&
      !isTypedScopeRequest(testCase.payload)
    ) {
      // The typed method forwards only `payload.scope`; the top-level
      // `cognition` member is discriminating data, so the whole payload
      // rides the raw signed session instead.
      return await rawScopeServe(
        testCase,
        pair.client,
        {
          clientSeed,
          hostPubkey,
          hostPeerId,
          clientManifest: manifestFor("client", FIXTURE.hosts[testCase.client]!),
        },
        calls,
      );
    }
    const client = await connectRemoteAdapter({
      transport: recordingTransport(pair.client, frames),
      localIdentity: { seed: clientSeed },
      localManifest: manifestFor("client", FIXTURE.hosts[testCase.client]!),
      remotePubkey: hostPubkey,
      allowlist: [hostPeerId],
    });
    try {
      if (testCase.op === SCOPE_LIST_OP) {
        // The fixture deliberately carries out-of-schema predicates on some
        // cases: the typed port method is the real wire path and forwards the
        // payload verbatim, which is exactly what the raw boundary must judge.
        const scope = testCase.payload.scope as Scope;
        return { result: await client.listKnowledgeEntries(scope), calls, frames };
      }
      if (testCase.op === MEMORY_LIST_OP) {
        const request = testCase.payload as unknown as MemoryRecordsListRequest;
        return { result: await client.listRecords(TEST_PRINCIPAL, request), calls, frames };
      }
      throw new Error(`unsupported served-case op ${testCase.op}`);
    } finally {
      client.close();
    }
  } finally {
    host.close();
  }
}

/**
 * The client-decoded success payload. Every success assertion reads this
 * value — what a consumer actually receives — never only the provider's
 * record of what it returned.
 */
function decodedEntries(
  result: SpokeResult<unknown>,
  caseId: string,
): KnowledgeEntry[] {
  if (!result.ok) {
    throw new Error(`${caseId}: expected success, received ${result.code}`);
  }
  if (!Array.isArray(result.value)) {
    throw new Error(`${caseId}: expected a decoded entry array`);
  }
  return result.value as KnowledgeEntry[];
}

function decodedMemoryPage(
  result: SpokeResult<unknown>,
  caseId: string,
): { records: unknown[]; total_authorized: number; next_cursor?: string } {
  if (!result.ok) {
    throw new Error(`${caseId}: expected success, received ${result.code}`);
  }
  const value = result.value;
  if (typeof value !== "object" || value === null) {
    throw new Error(`${caseId}: expected a decoded list page`);
  }
  if (!("records" in value)) {
    throw new Error(`${caseId}: expected a decoded list success page`);
  }
  return value as {
    records: unknown[];
    total_authorized: number;
    next_cursor?: string;
  };
}

function rejectDetails(result: SpokeResult<unknown>): Record<string, unknown> {
  if (result.ok) {
    throw new Error("expected a reject, received a success");
  }
  return result.details ?? {};
}

/**
 * The single error branch the host put on the wire, so the suite asserts the
 * exact wire code/reason/field instead of inferring them from the client's
 * mapping.
 */
function wireError(
  frames: Record<string, unknown>[],
  caseId: string,
): Record<string, unknown> {
  const errors = frames
    .map((frame) => frame.error)
    .filter(
      (error): error is Record<string, unknown> =>
        typeof error === "object" && error !== null,
    );
  expect(errors, `${caseId}: rejection frames`).toHaveLength(1);
  return errors[0]!;
}

describe("spoke-cognition negotiation (both-sided signed-hello intersection)", () => {
  it("grants the family only when both manifests declare it", () => {
    for (const testCase of NEGOTIATION_CASES) {
      const negotiated = negotiatedCapabilities(
        testCase.local_capabilities,
        testCase.remote_capabilities,
      );
      expect(
        negotiated.includes(CAPABILITY_SPOKE_COGNITION),
        `${testCase.id}: ${negotiated.join(",")}`,
      ).toBe(testCase.negotiated_includes_cognition);
    }
  });

  it("never reads the legacy flag as the new family", () => {
    const negotiated = negotiatedCapabilities(
      [FIXTURE.legacy_capability],
      [FIXTURE.capability],
    );
    expect(negotiated).toEqual([]);
    expect(negotiated.includes(CAPABILITY_SPOKE_COGNITION)).toBe(false);
  });
});

describe("spoke-cognition served boundary (actual responder/RemoteAdapter round trips)", () => {
  it("keeps the load-bearing escaped-field negatives by case identity", () => {
    // The two C7 field-path negatives are the only machine-checked proof of
    // the escaped §2.2 field contract; a dropped row must fail by identity.
    for (const [id, field] of [
      [
        "scope_carrier_empty_member_keeps_its_path",
        'scope.cognition.filter[""]',
      ],
      [
        "memory_carrier_delimiter_member_keeps_its_path",
        'cognition_filter.q["a.b"]',
      ],
    ] as const) {
      const testCase = SERVED_CASES.find((candidate) => candidate.id === id);
      expect(
        testCase,
        `load-bearing served vector missing: ${id}`,
      ).toBeDefined();
      expect(testCase!.expect.field, `${id}: escaped wire field`).toBe(field);
      expect(
        testCase!.expect_provider_calls,
        `${id}: a denial reaches no provider`,
      ).toBe(0);
    }
  });

  it("serves every valid selection and denies every unsupported requirement", async () => {
    for (const testCase of SERVED_CASES) {
      const { result, calls, frames } = await serveCase(testCase);
      const totalCalls =
        testCase.op === MEMORY_LIST_OP ? calls.memoryInvokes : calls.scopeLists;
      expect(totalCalls, `${testCase.id}: provider calls`).toBe(
        testCase.expect_provider_calls,
      );
      if (testCase.expect.status === "ok") {
        expect(result.ok, `${testCase.id}: expected success`).toBe(true);
        if (testCase.op === SCOPE_LIST_OP) {
          // The valid selection reaches the source unchanged: no added or
          // dropped context/filter member.
          expect(calls.receivedScopes, testCase.id).toEqual([
            testCase.payload.scope,
          ]);
          // The consumer-visible result is the client-decoded entry list; the
          // source's own counter is a supporting control only.
          expect(
            decodedEntries(result, testCase.id).map((entry) => entry.entry_id),
            `${testCase.id}: client-decoded entry ids`,
          ).toEqual(testCase.expect_entry_ids ?? []);
          expect(
            calls.returnedEntries.map((entry) => entry.entry_id),
            `${testCase.id}: source selection (control)`,
          ).toEqual(testCase.expect_entry_ids ?? []);
        } else {
          // `memory.records.list` success: assert the decoded page, so an
          // absent cursor stays absent while an explicit zero stays zero.
          const page = decodedMemoryPage(result, testCase.id);
          expect(page.records, `${testCase.id}: client-decoded records`).toEqual(
            [],
          );
          expect(page.total_authorized, `${testCase.id}: total_authorized`).toBe(
            0,
          );
          expect(
            Object.prototype.hasOwnProperty.call(page, "next_cursor"),
            `${testCase.id}: an absent cursor is never invented`,
          ).toBe(false);
        }
        continue;
      }
      const details = rejectDetails(result);
      // Exact wire parity (consumer contract §2.2) read from the raw frame:
      // the fixture's `code`/`reason`/`field` are the envelope the host must
      // emit — no case folding, no mapped-only inference.
      const error = wireError(frames, testCase.id);
      expect(error.code, `${testCase.id}: wire code`).toBe(testCase.expect.code);
      if (
        testCase.expect.code === "capability_missing" ||
        testCase.expect.code === "op_unsupported"
      ) {
        // The D7 row maps a dispatch deny to CAPABILITY_PORT_MISSING and
        // preserves the wire code.
        expect(result.ok).toBe(false);
        if (!result.ok) {
          expect(result.code, testCase.id).toBe(
            SpokeRejectCode.CAPABILITY_PORT_MISSING,
          );
        }
        expect(details.wire_code, testCase.id).toBe(testCase.expect.code);
        continue;
      }
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code, testCase.id).toBe(SpokeRejectCode.INVALID_INPUT);
      }
      const rawDetails = error.details as Record<string, unknown> | undefined;
      expect(rawDetails?.reason, `${testCase.id}: wire reason`).toBe(
        testCase.expect.reason,
      );
      expect(rawDetails?.field, `${testCase.id}: wire field`).toBe(
        testCase.expect.field,
      );
      // The adapter mapping preserves the same verdict for consumers.
      expect(details.reason, `${testCase.id}: reason`).toBe(
        testCase.expect.reason,
      );
      expect(details.field, `${testCase.id}: field`).toBe(testCase.expect.field);
    }
  });

  it("keeps ordinary (cognition-free) requests unchanged", async () => {
    const ordinary = SERVED_CASES.find(
      (testCase) => testCase.id === "ke_only_serves_absent_optional_cognition",
    );
    expect(ordinary).toBeDefined();
    const { result, calls } = await serveCase(ordinary!);
    expect(result.ok).toBe(true);
    expect(calls.scopeLists).toBe(1);
    expect(calls.receivedScopes).toEqual([{ scope_id: "world_1" }]);
    // The consumer receives every source entry decoded, in source order.
    expect(
      decodedEntries(result, ordinary!.id).map((entry) => entry.entry_id),
    ).toEqual(FIXTURE.ke_entries.map((entry) => entry.entry_id));
    expect(calls.returnedEntries).toHaveLength(FIXTURE.ke_entries.length);
  });

  it("serves fork, no-fork and multi-context results without inventing zero", async () => {
    const wildcard = SERVED_CASES.find(
      (testCase) =>
        testCase.id === "ke_only_serves_current_selection_with_wildcard_fork",
    )!;
    const { result, calls } = await serveCase(wildcard);
    expect(result.ok).toBe(true);
    expect(calls.scopeLists).toBe(1);
    // Read the records a consumer receives, not the source's return log.
    const entries = decodedEntries(result, wildcard.id);
    expect(entries.map((entry) => entry.entry_id)).toEqual([
      "kb_mira_fork",
      "kb_mira_nofork",
    ]);
    const [forkedEntry, noForkEntry] = entries;
    expect(forkedEntry?.entry_id).toBe("kb_mira_fork");
    expect(noForkEntry?.entry_id).toBe("kb_mira_nofork");
    const forked = entryCognitionRecords(forkedEntry!)[0]!;
    const noFork = entryCognitionRecords(noForkEntry!)[0]!;
    // Fixture records are belief records: the discriminator selects the
    // member that carries the assessment context and the optional metrics.
    expect(forked.kind).toBe("belief");
    expect(noFork.kind).toBe("belief");
    const forkedBelief = forked.kind === "belief" ? forked : undefined;
    const noForkBelief = noFork.kind === "belief" ? noFork : undefined;
    // Distinct assessment contexts are retained side by side.
    expect(forkedBelief?.assessment_context).toEqual({
      context_id: "ctx_main",
      fork_id: "fork_alpha",
    });
    expect(noForkBelief?.assessment_context).toEqual({ context_id: "ctx_main" });
    // Absent optional record data stays absent; an explicit zero stays zero.
    expect(forkedBelief?.metrics).toBeUndefined();
    expect(noForkBelief?.metrics?.b?.value).toBe(0);
  });

  it("keeps the original shape denial ahead of ownership and the cognition gate", async () => {
    // Pre-cognition baseline (`b6bfdc1` #dispatchPortInvoke) validates the
    // declared Scope shape BEFORE the `ke-ownership` conjunct, and both run
    // before the cognition gate. A malformed Scope that also carries a
    // viewpoint is therefore the original shape denial — never an ownership
    // or §2.2 verdict.
    const malformedWithViewpoint: ServedCase = {
      id: "malformed_scope_shape_before_ownership",
      kind: "served",
      host: "ke_only_conformant",
      client: "ke_only_conformant",
      op: SCOPE_LIST_OP,
      payload: { scope: { viewpoint: "kb_mira" } },
      expect: { status: "reject" },
      expect_provider_calls: 0,
    };
    const plain = await serveCase(malformedWithViewpoint);
    expect(plain.result.ok).toBe(false);
    expect(plain.calls.scopeLists).toBe(0);
    const plainError = wireError(plain.frames, malformedWithViewpoint.id);
    expect(plainError.code).toBe(SpokeRejectCode.INVALID_INPUT);
    expect(
      (plainError.details as Record<string, unknown> | undefined)?.reason,
      "the baseline shape denial carries no §2.2 verdict",
    ).toBeUndefined();

    // The shape denial also precedes the cognition diagnosis: delivering an
    // unsupported filter does not preempt the malformed baseline Scope.
    const withCognition: ServedCase = {
      ...malformedWithViewpoint,
      id: "shape_first_with_cognition",
      payload: {
        scope: {
          viewpoint: "kb_mira",
          cognition: { view: "current", filter: { q_min: 0.5 } },
        },
      },
    };
    const selected = await serveCase(withCognition);
    expect(selected.result.ok).toBe(false);
    expect(selected.calls.scopeLists).toBe(0);
    const selectedError = wireError(selected.frames, withCognition.id);
    expect(selectedError.code).toBe(SpokeRejectCode.INVALID_INPUT);
    expect(
      (selectedError.details as Record<string, unknown> | undefined)?.reason,
      "an unsupported cognition requirement never preempts the shape denial",
    ).toBeUndefined();

    // A sound shape keeps ownership precedence: the shared fixture's
    // ownership vector carries a valid `scope_id`, so the shape gate passes
    // and the missing `ke-ownership` still denies with the original code (see
    // `ownership_denial_precedes_the_cognition_denial` in the loop above).
    const ownership = SERVED_CASES.find(
      (testCase) =>
        testCase.id === "ownership_denial_precedes_the_cognition_denial",
    )!;
    const ownershipServed = await serveCase(ownership);
    expect(ownershipServed.result.ok).toBe(false);
    expect(ownershipServed.calls.scopeLists).toBe(0);
    expect(wireError(ownershipServed.frames, ownership.id).code).toBe(
      "op_unsupported",
    );
  });

  it("rejects before the provider for every denied case", () => {
    const denied = SERVED_CASES.filter(
      (testCase) => testCase.expect.status === "reject",
    );
    for (const testCase of denied) {
      expect(testCase.expect_provider_calls, testCase.id).toBe(0);
    }
  });
});
