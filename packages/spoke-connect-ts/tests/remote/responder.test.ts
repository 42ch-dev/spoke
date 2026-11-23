/**
 * note: `connectResponder` (Task 2 of tools-connect-reverse-invoke) — loopback
 * pair: the productized responder (`connectResponder` on the server end,
 * frozen contract `tool-contracts.md` §6) ↔ a T1 tool-serving
 * `RemoteAdapter` (dialer).
 *
 * Asserts, per the plan:
 * (a) the ported demo responder recipe: allowlist-first → hello verify →
 *     nonce single-use → dial-bound responder hello → signed session
 *     snapshot; gate = peek → verify → advance (serialized), with
 *     auth-before-advance;
 * (b) `port.*` serving against an injected async `BaselinePorts` via the
 *     D4 catalogue, with the dispatch-deny branch for unknown port methods
 *     AND for absent `ports` (documented);
 * (c) the `invokeTool` reverse face: outbound counter, request signing,
 *     send-tail wire-order serialization, response correlation +
 *     envelope-auth verify, per-waiter timeout (waiter-only), and the
 *     deferred-send poison-close mirror (a waiter that settles while its
 *     send is queued closes the session);
 * (d) carried-over demo behaviors: the empty-intersection snapshot
 *     fallback still establishes, and an unparseable inbound closes the
 *     connection.
 */

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

import type {
  ExtractRequest,
  MemoryCandidatesReviewRequest,
  MemoryCaptureRequest,
  MemoryDescribeRequest,
  MemoryExportsGetRequest,
  MemoryImportsStageRequest,
  MemoryReceiptsGetRequest,
  ComputeRequest,
  ConnectHello,
  ConnectInvokeRequest,
  ForkId,
  HostCapabilityManifest,
  KnowledgeEntry,
  ProjectRequest,
  Scope,
  ToolDescriptor,
} from "@42ch/spoke-schemas";
import {
  orchestrateExtract,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  validateManifestTools,
  type BaselinePorts,
  type ExtractionPort,
  type SpokeResult,
} from "@42ch/spoke-operations";
import {
  connectRemoteAdapter,
  isConnectInvokeResponse,
  loopbackTransportPair,
  RemoteAdapter,
  type EnvelopeBytes,
  type Transport,
} from "@42ch/spoke-connect/remote";
import {
  asBaselineOnly,
  ToyWorldAdapter,
} from "@42ch/spoke-fixture-toy-world";
import { CAPABILITY_KE_EXTRACTION } from "../../src/core/dispatch.js";
import {
  CAPABILITY_KE_OWNERSHIP,
  testActiveServeBudgetWaits,
  validateExtractRequestPayload,
  validateScopeOpPayload,
} from "../../src/remote/responder.js";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import {
  authenticateInvokeRequest,
  authenticateInvokeResponse,
} from "../../src/core/envelope-auth.js";
import { verifyHelloEd25519 } from "../../src/core/hello.js";
import { generateNonce } from "../../src/core/nonce.js";
import { signHelloEd25519 } from "../../src/core/hello.js";
import { schemaConformantManifest } from "../fixtures/golden.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  connectResponder,
  knowledgeOnlyServePorts,
  withKnowledge,
  type ConnectResponder,
  type ConnectResponderState,
  type RemoteExtractService,
  type RemoteKnowledgeFace,
  type RemoteMemoryService,
  type RemoteServePorts,
} from "@42ch/spoke-connect/remote";

/**
 * note: the scope the default fixture knowledge face admits; `toServePorts`
 * forwards baseline get/put through it and matches list wire scopes.
 */
const ADMITTED_SCOPE: Scope = { scope_id: "toy-scope-001" };

function knowledgeFaceAdmitting(
  ports: BaselinePorts,
  scope: Scope = ADMITTED_SCOPE,
): RemoteKnowledgeFace {
  return {
    store: {
      getScopedKnowledgeEntry: (_scope, entryId) =>
        ports.getKnowledgeEntry(entryId),
      putScopedKnowledgeEntry: (_scope, entry, expectedBaseRevision) =>
        ports.putKnowledgeEntry(entry, expectedBaseRevision),
      listScopedKnowledgeEntries: (admitted) =>
        ports.listKnowledgeEntries(admitted),
    },
    admission: () => spokeOk(scope),
  };
}

/**
 * note: legacy fixture adapters carry the extract/memory service fields on the
 * same object as the baseline port methods; the responder now takes a face
 * object, so test fixtures normalize once here.
 */
function toServePorts(
  ports: BaselinePorts & Partial<RemoteExtractService> & Partial<RemoteMemoryService>,
  admitted?: Scope,
): RemoteServePorts & Partial<RemoteExtractService> & Partial<RemoteMemoryService> {
  return {
    baseline: ports,
    knowledge: knowledgeFaceAdmitting(ports, admitted ?? ADMITTED_SCOPE),
    extract: ports.extract,
    invoke: ports.invoke,
  };
}

/** note: fixture seed — base+i, all values within byte range for base ≤ 0xe0. */
function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

const ADD_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.math.add",
  op: "tools.math.add",
  description: "Add two numbers",
  input: {},
  output: {},
};

const ECHO_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.echo.echo",
  op: "tools.echo.echo",
  description: "Echo the arguments back",
  input: {},
  output: {},
};

const BOOM_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.echo.boom",
  op: "tools.echo.boom",
  description: "Handler that throws",
  input: {},
  output: {},
};

/** note: namespaces own the tool namespaces; every tool capability ∈ capabilities[]. */
function toolManifest(hostId: string): HostCapabilityManifest {
  return {
    ...schemaConformantManifest(),
    host_id: hostId,
    namespaces: ["toy_world", "math", "echo"],
    capabilities: [
      "spoke-baseline",
      // why: these fixtures serve the knowledge face, so the session negotiates
      // `spoke-knowledge` explicitly (no flag implies it).
      "spoke-knowledge",
      "tools.math.add",
      "tools.echo.echo",
      "tools.echo.boom",
    ],
    tools: [ADD_DESCRIPTOR, ECHO_DESCRIPTOR, BOOM_DESCRIPTOR],
  };
}

function addHandler(calls: { args: Record<string, unknown> }[]): {
  (args: Record<string, unknown>): Promise<SpokeResult<unknown>>;
} {
  return async (args) => {
    calls.push({ args });
    return spokeOk({ sum: (args.a as number) + (args.b as number) });
  };
}

interface DialResponderOptions {
  clientManifest?: HostCapabilityManifest;
  responderManifest?: HostCapabilityManifest;
  ports?: BaselinePorts &
    Partial<RemoteExtractService> &
    Partial<RemoteMemoryService>;
  /** note: the scope the default knowledge face admits (list wire scopes must match). */
  admittedScope?: Scope;
  /** note: bound on the RESPONDER's reverse-invoke waiters, ms. */
  responderTimeoutMs?: number;
  /** note: bound on the CLIENT's dial + invoke waiters, ms. */
  clientTimeoutMs?: number;
  clientTransport?: (clientEnd: Transport) => Transport;
  responderTransport?: (serverEnd: Transport) => Transport;
}

/**
 * note: pairs `connectResponder` (server end) with a real T1
 * `connectRemoteAdapter` (client end); the responder's handshake runs in
 * the background and the client's dial is the synchronization point.
 */
async function dialWithResponder(
  options: DialResponderOptions = {},
): Promise<{
  responder: ConnectResponder;
  client: RemoteAdapter;
  pair: ReturnType<typeof loopbackTransportPair>;
  peerIdResponder: string;
  peerIdClient: string;
  seedClient: Uint8Array;
}> {
  const seedResponder = seed(0xa0);
  const seedClient = seed(0x10);
  const pubkeyResponder = getPublicKeyEd25519(seedResponder);
  const pubkeyClient = getPublicKeyEd25519(seedClient);
  const peerIdResponder = derivePeerIdFromEd25519Pubkey(pubkeyResponder);
  const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

  const clientManifest = options.clientManifest ?? toolManifest("test-client");
  const responderManifest =
    options.responderManifest ?? toolManifest("test-responder");
  // why: fixture hygiene — both manifests must satisfy the manifest-tools rules.
  for (const manifest of [clientManifest, responderManifest]) {
    const validated = validateManifestTools(manifest);
    expect(validated.ok, `manifest tools must validate: ${manifest.host_id}`).toBe(
      true,
    );
  }

  const pair = loopbackTransportPair();
  const responder = await connectResponder({
    transport: options.responderTransport?.(pair.server) ?? pair.server,
    identity: { seed: seedResponder },
    manifest: responderManifest,
    allowlist: [peerIdClient],
    peerKeys: { [peerIdClient]: pubkeyClient },
    ports: options.ports && toServePorts(options.ports, options.admittedScope),
    invokeTimeoutMs: options.responderTimeoutMs,
  });
  const client = await connectRemoteAdapter({
    transport: options.clientTransport?.(pair.client) ?? pair.client,
    localIdentity: { seed: seedClient },
    localManifest: clientManifest,
    remotePubkey: pubkeyResponder,
    allowlist: [peerIdResponder],
    invokeTimeoutMs: options.clientTimeoutMs,
  });
  return { responder, client, pair, peerIdResponder, peerIdClient, seedClient };
}

function decodeWire(bytes: EnvelopeBytes): Record<string, unknown> {
  return decodeJsonMessage(bytes) as Record<string, unknown>;
}

/** note: real wall-clock wait — only for real-timer integration fixtures. */
function delay(ms: number): Promise<void> {
  // why: the project's TS lib target predates `Promise.withResolvers` (ES2024),
  // so executor form is required here.
  return new Promise<void>((resolve) => {
    setTimeout(resolve, ms);
  });
}

/** note: bounded poll (5s) for an async state transition. */
async function untilState(
  target: { state: ConnectResponderState | string },
  state: string,
  what: string,
): Promise<void> {
  const deadline = Date.now() + 5000;
  while (target.state !== state) {
    if (Date.now() >= deadline) {
      throw new Error(`${what} never reached ${state} (state ${target.state})`);
    }
    await delay(10);
  }
}

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

function decodeEnvelope(bytes: EnvelopeBytes): unknown {
  return decodeJsonMessage(bytes);
}

/** note: starts a responder WITHOUT dialing — raw-wire tests drive the wire. */
async function startRawResponder(options: {
  ports?: BaselinePorts;
  manifest?: HostCapabilityManifest;
  /** note: responder serve budget + reverse-invoke waiter deadline, ms. */
  invokeTimeoutMs?: number;
} = {}): Promise<{
  responder: ConnectResponder;
  clientEnd: Transport;
  seedClient: Uint8Array;
  pubkeyResponder: Uint8Array;
  peerIdResponder: string;
  peerIdClient: string;
}> {
  const seedResponder = seed(0xa0);
  const seedClient = seed(0x10);
  const pubkeyResponder = getPublicKeyEd25519(seedResponder);
  const pubkeyClient = getPublicKeyEd25519(seedClient);
  const peerIdResponder = derivePeerIdFromEd25519Pubkey(pubkeyResponder);
  const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);
  const pair = loopbackTransportPair();
  const responder = await connectResponder({
    transport: pair.server,
    identity: { seed: seedResponder },
    manifest: options.manifest ?? toolManifest("test-responder"),
    allowlist: [peerIdClient],
    peerKeys: { [peerIdClient]: pubkeyClient },
    ports: toServePorts(options.ports ?? ToyWorldAdapter.withCommittedFixtures()),
    invokeTimeoutMs: options.invokeTimeoutMs,
  });
  return {
    responder,
    clientEnd: pair.client,
    seedClient,
    pubkeyResponder,
    peerIdResponder,
    peerIdClient,
  };
}

/** note: raw initiator handshake — the real library client is exercised elsewhere. */
async function rawHandshake(
  client: Transport,
  options: {
    seed: Uint8Array;
    manifest: HostCapabilityManifest;
    pubkeyResponder: Uint8Array;
    peerIdResponder: string;
  },
): Promise<{ session_id: string }> {
  const initiatorNonce = generateNonce();
  await client.send(
    encodeEnvelope(
      await signHelloEd25519(options.seed, initiatorNonce, options.manifest),
    ),
  );
  const serverHello = decodeEnvelope(await client.recv()) as ConnectHello;
  await verifyHelloEd25519(
    options.pubkeyResponder,
    options.peerIdResponder,
    serverHello,
    initiatorNonce,
  );
  const sessionDoc = decodeEnvelope(await client.recv()) as {
    session_id: string;
  };
  return { session_id: sessionDoc.session_id };
}

/** note: signs a raw wire `ConnectInvokeRequest` over the locked 5-field set. */
async function signInvokeRequest(
  seed: Uint8Array,
  request: {
    session_id: string;
    sequence: number;
    request_id: string;
    op: string;
    /** note: wire any-JSON payload (`OpaqueJson`), including JSON null. */
    payload: unknown;
  },
): Promise<ConnectInvokeRequest> {
  return authenticateInvokeRequest(seed, request);
}

const MIRA_ENTRY_ID = "kb_tw_mira";

describe("connectResponder handshake and session", () => {
  it(
    "establishes a session with a real connectRemoteAdapter dial (discovery after auth)",
    async () => {
      const { responder, client, peerIdResponder, peerIdClient } =
        await dialWithResponder();
      try {
        expect(responder.state).toBe("Established");
        expect(client.state).toBe("Established");
        expect(responder.sessionId.length).toBeGreaterThan(0);
        expect(responder.sessionId).toBe(client.sessionId);
        // note: session peer binding — the responder's remote peer is the dialer.
        expect(responder.remotePeerId).toBe(peerIdClient);
        expect(client.remotePeerId).toBe(peerIdResponder);
        // note: discovery after auth — the authenticated hello `host` is the
        // source, so the responder sees the dialer's tools[] only once the
        // signed-hello handshake completed.
        expect(responder.remoteManifest).toEqual(toolManifest("test-client"));
        expect(responder.remoteManifest.tools?.map((t) => t.capability_id)).toEqual([
          "tools.math.add",
          "tools.echo.echo",
          "tools.echo.boom",
        ]);
        expect(client.remoteManifest).toEqual(toolManifest("test-responder"));
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "rejects a non-allowlisted peer during the handshake (fail-closed)",
    async () => {
      const seedStranger = seed(0x70);
      const pair = loopbackTransportPair();
      const seedResponder = seed(0xa0);
      const pubkeyResponder = getPublicKeyEd25519(seedResponder);
      const peerIdResponder = derivePeerIdFromEd25519Pubkey(pubkeyResponder);
      const peerIdStranger = derivePeerIdFromEd25519Pubkey(
        getPublicKeyEd25519(seedStranger),
      );
      const strangerManifest: HostCapabilityManifest = {
        ...toolManifest("test-stranger"),
        host_id: "test-stranger",
      };
      const responder = await connectResponder({
        transport: pair.server,
        identity: { seed: seedResponder },
        manifest: toolManifest("test-responder"),
        allowlist: [peerIdStranger], // note: allow the STRANGER (client-side trusts the server)
        peerKeys: {
          [peerIdStranger]: getPublicKeyEd25519(seedStranger),
        },
        ports: { baseline: new ToyWorldAdapter() },
      });
      try {
        // why: the server-side allowlist rejects the hello and closes the
        // transport, failing the dial fast.
        await expect(
          connectRemoteAdapter({
            transport: pair.client,
            localIdentity: { seed: seed(0x10) },
            localManifest: strangerManifest,
            remotePubkey: pubkeyResponder,
            allowlist: [peerIdResponder],
          }),
        ).rejects.toThrow();
        await untilState(responder, "Closed", "responder");
      } finally {
        responder.close();
      }
    },
    15000,
  );
});

describe("connectResponder port serving (D4 catalogue)", () => {
  it(
    "round-trips port.* ops through the responder into the injected BaselinePorts",
    async () => {
      const { responder, client } = await dialWithResponder({
        ports: ToyWorldAdapter.withCommittedFixtures(),
      });
      try {
        const mira = await client.getKnowledgeEntry(MIRA_ENTRY_ID);
        expect(mira.ok).toBe(true);
        if (mira.ok) {
          expect(mira.value.entry_id).toBe(MIRA_ENTRY_ID);
        }

        // note: create (expected_base_revision null), then a compare-and-swap
        // update over the wire — the toy-world store treats an absent revision
        // as 0, so base 0 accepts the update.
        const compass: KnowledgeEntry = {
          schema_version: 1,
          entry_id: "test-harbor/item/compass",
          entry_type: "item",
          canonical_name: "Compass",
          status: "provisional",
          body: { summary: "A brass compass." },
          extensions: {},
        };
        const created = await client.putKnowledgeEntry(compass, null);
        expect(created.ok).toBe(true);
        if (created.ok) {
          expect(created.value.entry_id).toBe(compass.entry_id);
        }
        const updated = await client.putKnowledgeEntry(
          { ...compass, status: "confirmed" },
          0,
        );
        expect(updated.ok).toBe(true);
        if (updated.ok) {
          expect(updated.value.status).toBe("confirmed");
        }

        // note: negative OCC over the wire — re-creating an existing entry rejects
        // REVISION_CONFLICT through the responder's error branch.
        const conflicted = await client.putKnowledgeEntry(compass, null);
        expect(conflicted.ok).toBe(false);
        if (!conflicted.ok) {
          expect(conflicted.code).toBe(SpokeRejectCode.REVISION_CONFLICT);
        }

        const listed = await client.listKnowledgeEntries({
          scope_id: "toy-scope-001",
        });
        expect(listed.ok).toBe(true);
        if (listed.ok) {
          expect(
            listed.value.some((entry) => entry.entry_id === compass.entry_id),
          ).toBe(true);
        }

        const peers = await client.listPeerHostCapabilityManifests();
        expect(peers.ok).toBe(true);
        if (peers.ok) {
          expect(peers.value.map((m) => m.host_id)).toEqual(["host_tw_peer"]);
        }

        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "validates the nullable relation.put CAS fence before the provider (qc1 F-004)",
    async () => {
      // why: the baseline Relation dispatch used to cast
      // `expected_base_revision` straight to the provider; Rust refuses an
      // out-of-domain parse before constructing the call. The inclusive
      // `2^53` endpoint must reach the provider, the next representable value
      // (`2^53 + 2`) must refuse `INVALID_INPUT` with zero provider calls.
      const baseline = asBaselineOnly(ToyWorldAdapter.withCommittedFixtures());
      const bases: (number | null)[] = [];
      const ports: BaselinePorts = {
        ...baseline,
        putRelation: async (relation, expectedBaseRevision) => {
          bases.push(expectedBaseRevision);
          return spokeOk(relation);
        },
      };
      const { responder, clientEnd, seedClient, pubkeyResponder, peerIdResponder } =
        await startRawResponder({ ports });
      try {
        const { session_id: sessionId } = await rawHandshake(clientEnd, {
          seed: seedClient,
          manifest: toolManifest("test-client"),
          pubkeyResponder,
          peerIdResponder,
        });
        const relation = {
          schema_version: 1,
          relation_id: "rel_boundary",
          relation_type: "related_to",
          from_id: "kb_from",
          to_id: "kb_to",
          extensions: {},
        };
        const accepted = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 0,
          request_id: "relation-put-ceiling",
          op: "port.relation.put",
          payload: { relation, expected_base_revision: 2 ** 53 },
        });
        await clientEnd.send(encodeEnvelope(accepted));
        const acceptedResponse = decodeEnvelope(await clientEnd.recv()) as {
          error?: unknown;
        };
        expect(acceptedResponse.error).toBeUndefined();
        expect(bases).toEqual([2 ** 53]);

        const refused = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 1,
          request_id: "relation-put-over-ceiling",
          op: "port.relation.put",
          payload: { relation, expected_base_revision: 2 ** 53 + 2 },
        });
        await clientEnd.send(encodeEnvelope(refused));
        const refusedResponse = decodeEnvelope(await clientEnd.recv()) as {
          error: { code: string };
        };
        expect(refusedResponse.error.code).toBe(SpokeRejectCode.INVALID_INPUT);
        expect(bases).toEqual([2 ** 53]);

        // note: an unbounded nested `relation.revision` is refused before the
        // provider (qc2); Rust's typed decode refuses the same value.
        const badRevision = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 2,
          request_id: "relation-put-bad-revision",
          op: "port.relation.put",
          payload: {
            relation: { ...relation, revision: 2 ** 53 + 2 },
            expected_base_revision: 0,
          },
        });
        await clientEnd.send(encodeEnvelope(badRevision));
        const badRevisionResponse = decodeEnvelope(await clientEnd.recv()) as {
          error: { code: string };
        };
        expect(badRevisionResponse.error.code).toBe(SpokeRejectCode.INVALID_INPUT);
        expect(bases).toEqual([2 ** 53]);

        // note: Rust refuses a missing CAS member; absence must not become
        // `null` (create-only) on the TS path (qc2).
        const missingCas = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 3,
          request_id: "relation-put-missing-cas",
          op: "port.relation.put",
          payload: { relation },
        });
        await clientEnd.send(encodeEnvelope(missingCas));
        const missingCasResponse = decodeEnvelope(await clientEnd.recv()) as {
          error: { code: string };
        };
        expect(missingCasResponse.error.code).toBe(SpokeRejectCode.INVALID_INPUT);
        expect(bases).toEqual([2 ** 53]);
      } finally {
        responder.close();
      }
    },
    15000,
  );

  it(
    "answers an unknown port method with the dispatch-deny branch (op_unsupported)",
    async () => {
      const { responder, clientEnd, seedClient, pubkeyResponder, peerIdResponder } =
        await startRawResponder();
      try {
        const { session_id: sessionId } = await rawHandshake(clientEnd, {
          seed: seedClient,
          manifest: toolManifest("test-client"),
          pubkeyResponder,
          peerIdResponder,
        });
        // why: a wire-valid, properly signed invoke for an op outside the D4
        // catalogue — the dispatch gate denies it (no core row, no product
        // map row) with the existing `op_unsupported` error branch.
        const unknownOp = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 0,
          request_id: "unknown-port-op",
          op: "port.nope",
          payload: {},
        });
        await clientEnd.send(encodeEnvelope(unknownOp));
        const response = decodeEnvelope(await clientEnd.recv()) as {
          error: { code: string; message: string };
          signature: string;
        };
        expect(response.error.code).toBe("op_unsupported");
        expect(response.error.message).toContain("port.nope");
        expect(response.signature).toHaveLength(86);
      } finally {
        responder.close();
      }
    },
    15000,
  );

  it(
    "denies port.* invokes with the dispatch-deny branch when ports are absent",
    async () => {
      // why: no `ports` injected — the capability gate passes (spoke-baseline is
      // negotiated) but there is no BaselinePorts to serve, so the responder
      // answers the dispatch-deny branch, mapped by the D7 invoker row.
      const { responder, client } = await dialWithResponder();
      try {
        const result = await client.getKnowledgeEntry(MIRA_ENTRY_ID);
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("no ports face configured"),
          details: { wire_code: "op_unsupported" },
        });
        expect(responder.state).toBe("Established");
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );
});

describe("connectResponder optional-port serving (gate → probe → serve/deny)", () => {
  /**
   * note: manifest declaring the optional families on top of the tool-carrying
   * baseline — `l2-computable` (project / compute) + `l5-fork`
   * (listForkTimelineEvents). Both dial peers carry it so the negotiated
   * intersection authorizes the optional ops on the responder's gate.
   */
  function optionalManifest(hostId: string): HostCapabilityManifest {
    return {
      ...toolManifest(hostId),
      // why: the tool-carrying baseline capabilities stay; the optional family
      // capability ids are appended.
      capabilities: [
        ...toolManifest(hostId).capabilities,
        "l2-computable",
        "l5-fork",
      ],
    };
  }

  const fullPorts = () => ToyWorldAdapter.withCommittedFixtures();

  const optionalDial = () =>
    dialWithResponder({
      ports: fullPorts(),
      clientManifest: optionalManifest("test-client"),
      responderManifest: optionalManifest("test-responder"),
    });

  it(
    "round-trips project through the responder (port.computable.project, snake_case payload) while baseline serving stays green",
    async () => {
      const { responder, client } = await optionalDial();
      try {
        // note: baseline smoke in the SAME session — the catalogue extension must
        // not disturb the baseline six (serving-order preservation).
        const mira = await client.getKnowledgeEntry(MIRA_ENTRY_ID);
        expect(mira.ok).toBe(true);
        if (mira.ok) {
          expect(mira.value.entry_id).toBe(MIRA_ENTRY_ID);
        }

        const request: ProjectRequest = {
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          state: { tide_level: 2.1, cargo_tons: 40 },
        };
        const result = await client.project(request);
        expect(result.ok).toBe(true);
        if (result.ok) {
          // note: the adapter echoes request ids and serves the committed
          // projection fixture — proves the payload carried the request
          // through the loopback and the responder delegated it.
          expect(result.value).toEqual({
            session_id: "sess_tw_dawn_arrival",
            entry_id: "kb_tw_harbor",
            computable: { tide_level: 2.4, cargo_tons: 38 },
          });
        }
        expect(responder.state).toBe("Established");
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "round-trips compute through the responder (port.computable.compute, settle response carries state)",
    async () => {
      const { responder, client } = await optionalDial();
      try {
        const request: ComputeRequest = {
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          computable: { tide_level: 2.5, cargo_tons: 37 },
          settle: true,
        };
        const result = await client.compute(request);
        expect(result.ok).toBe(true);
        if (result.ok) {
          expect(result.value).toEqual({
            session_id: "sess_tw_dawn_arrival",
            entry_id: "kb_tw_harbor",
            computable: { tide_level: 2.5, cargo_tons: 37 },
            state: { tide_level: 2.5, cargo_tons: 37 },
          });
        }
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "round-trips listForkTimelineEvents through the responder (port.fork.list_timeline_events, fork_id-scoped)",
    async () => {
      const { responder, client } = await optionalDial();
      try {
        const scope: Scope & { fork_id: ForkId } = {
          scope_id: "pkt_tw_scope",
          fork_id: "fork_tw_storm_branch",
        };
        const result = await client.listForkTimelineEvents(scope);
        expect(result.ok).toBe(true);
        if (result.ok) {
          // note: the committed fixtures carry exactly one fork-branch event —
          // proves the fork_id survived the loopback into the provider.
          expect(result.value).toHaveLength(1);
          expect(result.value[0]).toMatchObject({
            timeline_event_id: "evt_tw_harbor_storm_delay",
            fork_id: "fork_tw_storm_branch",
          });
        }
        const empty = await client.listForkTimelineEvents({
          scope_id: "pkt_tw_scope",
          fork_id: "fork_tw_unknown",
        });
        expect(empty.ok).toBe(true);
        if (empty.ok) {
          expect(empty.value).toEqual([]);
        }
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "denies port.computable.project when l2-computable is not negotiated (gate deny → op_unsupported → CAPABILITY_PORT_MISSING + wire_code)",
    async () => {
      // why: default manifests advertise spoke-baseline only ⇒ the negotiated
      // set lacks l2-computable ⇒ the responder's dispatch gate denies the
      // op even though the injected provider implements it (gate FIRST).
      const { responder, client } = await dialWithResponder({
        ports: fullPorts(),
      });
      try {
        const request: ProjectRequest = {
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          state: { tide_level: 2.1, cargo_tons: 40 },
        };
        const result = await client.project(request);
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("not authorized"),
          details: { wire_code: "op_unsupported" },
        });
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "denies port.computable.compute when l2-computable is not negotiated (gate deny → op_unsupported → CAPABILITY_PORT_MISSING + wire_code)",
    async () => {
      const { responder, client } = await dialWithResponder({
        ports: fullPorts(),
      });
      try {
        const request: ComputeRequest = {
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          computable: { tide_level: 2.5, cargo_tons: 37 },
          settle: true,
        };
        const result = await client.compute(request);
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("not authorized"),
          details: { wire_code: "op_unsupported" },
        });
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "denies port.fork.list_timeline_events when l5-fork is not negotiated (gate deny → op_unsupported → CAPABILITY_PORT_MISSING + wire_code)",
    async () => {
      const { responder, client } = await dialWithResponder({
        ports: fullPorts(),
      });
      try {
        const scope: Scope & { fork_id: ForkId } = {
          scope_id: "pkt_tw_scope",
          fork_id: "fork_tw_storm_branch",
        };
        const result = await client.listForkTimelineEvents(scope);
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("not authorized"),
          details: { wire_code: "op_unsupported" },
        });
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "denies all three optional ops when the capability is declared but the provider is baseline-only (probe deny → op_unsupported → CAPABILITY_PORT_MISSING + wire_code)",
    async () => {
      // why: both peers declare the optional families (gate passes) but the
      // injected ports object is a baseline-only view without the optional
      // methods — the responder's structural probe (gate → probe → deny)
      // answers the same dispatch-deny branch as absent ports.
      const { responder, client } = await dialWithResponder({
        ports: asBaselineOnly(ToyWorldAdapter.withCommittedFixtures()),
        clientManifest: optionalManifest("test-client"),
        responderManifest: optionalManifest("test-responder"),
      });
      try {
        const projectResult = await client.project({
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          state: { tide_level: 2.1, cargo_tons: 40 },
        });
        // note: probe deny (not the gate deny) — the capability gate passed and
        // the responder's structural probe reports the missing method.
        expect(projectResult).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining(
            "requires optional port method project",
          ),
          details: { wire_code: "op_unsupported" },
        });

        const computeResult = await client.compute({
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          computable: { tide_level: 2.5, cargo_tons: 37 },
          settle: true,
        });
        expect(computeResult).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining(
            "requires optional port method compute",
          ),
          details: { wire_code: "op_unsupported" },
        });

        const forkResult = await client.listForkTimelineEvents({
          scope_id: "pkt_tw_scope",
          fork_id: "fork_tw_storm_branch",
        });
        expect(forkResult).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining(
            "requires optional port method listForkTimelineEvents",
          ),
          details: { wire_code: "op_unsupported" },
        });

        const mira = await client.getKnowledgeEntry(MIRA_ENTRY_ID);
        expect(mira.ok).toBe(true);
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "serves computable ops and probe-denies fork ops on a mixed composite (computable face present, fork face absent)",
    async () => {
      // note: mixed host — the injected ports compose the baseline + l2-computable
      // faces without the l5-fork face; computable ops serve through the
      // present face, while the fork op probe-denies (the gate passes, so the
      // missing face is host misconfiguration).
      const full = ToyWorldAdapter.withCommittedFixtures();
      const mixedComposite = {
        ...asBaselineOnly(full),
        project: (request: ProjectRequest) => full.project(request),
        compute: (request: ComputeRequest) => full.compute(request),
      };
      const { responder, client } = await dialWithResponder({
        ports: mixedComposite,
        clientManifest: optionalManifest("test-client"),
        responderManifest: optionalManifest("test-responder"),
      });
      try {
        const projectResult = await client.project({
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          state: { tide_level: 2.1, cargo_tons: 40 },
        });
        expect(projectResult.ok).toBe(true);
        const forkResult = await client.listForkTimelineEvents({
          scope_id: "pkt_tw_scope",
          fork_id: "fork_tw_storm_branch",
        });
        expect(forkResult).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining(
            "requires optional port method listForkTimelineEvents",
          ),
          details: { wire_code: "op_unsupported" },
        });
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );
});

describe("connectResponder reverse invoke (invokeTool)", () => {
  it(
    "issues a reverse invoke served by the dialer's registered handler (signed result)",
    async () => {
      const calls: { args: Record<string, unknown> }[] = [];
      const { responder, client } = await dialWithResponder();
      try {
        client.registerToolHandler("tools.math.add", addHandler(calls));
        const result = await responder.invokeTool("tools.math.add", {
          a: 2,
          b: 3,
        });
        expect(result).toEqual({ ok: true, value: { sum: 5 } });
        // note: the dialer-side registered handler is what ran, with the
        // request's arguments object passed through.
        expect(calls).toEqual([{ args: { a: 2, b: 3 } }]);
        expect(responder.state).toBe("Established");
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "maps a deny (no handler) to CAPABILITY_PORT_MISSING with the wire code preserved",
    async () => {
      const { responder, client } = await dialWithResponder();
      try {
        // note: the tool IS negotiated but the dialer serves no handler for it —
        // fail-closed serving → op_unsupported → D7 mapping.
        const result = await responder.invokeTool("tools.math.add", {
          a: 1,
          b: 2,
        });
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("no handler registered"),
          details: { wire_code: "op_unsupported" },
        });
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "maps a deny (not negotiated) to CAPABILITY_PORT_MISSING (dispatch gate)",
    async () => {
      // why: the client manifest carries no tools, so the negotiated set lacks the
      // tool capability and the dialer's dispatch gate denies the reverse
      // invoke (frozen deny matrix: gate fail → op_unsupported).
      const noTools: HostCapabilityManifest = {
        ...schemaConformantManifest(),
        host_id: "test-client",
        capabilities: ["spoke-baseline"],
      };
      const { responder, client } = await dialWithResponder({
        clientManifest: noTools,
      });
      try {
        const result = await responder.invokeTool("tools.math.add", {
          a: 1,
          b: 2,
        });
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("not authorized"),
          details: { wire_code: "op_unsupported" },
        });
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "fails fast with INVALID_INPUT on a non-tool capability id (invokeTool grammar gate)",
    async () => {
      const { responder, client } = await dialWithResponder();
      try {
        const result = await responder.invokeTool("spoke-baseline", {});
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INVALID_INPUT,
          message: expect.stringContaining('must start with "tools." prefix'),
          details: { capability_id: "spoke-baseline" },
        });
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "rejects a reverse invokeTool whose dialer success payload lacks a result key (INTERNAL_ERROR kind transport)",
    async () => {
      // why: T2-M1 (folded) — the RESPONDER's reverse invokeTool { result } gate.
      // The dialer's serving path answers a signed success envelope whose
      // payload is NOT `{ result: <opaque JSON> }` — the frozen tool
      // success-payload gate must reject instead of surfacing
      // spokeOk(garbage). Wire-level injector on the CLIENT end: the
      // dialer's outbound RESPONSES to reverse invokes are re-signed with
      // malformed payloads (legitimately signed, so the responder's
      // envelope-auth verify passes and the { result } gate is what
      // rejects).
      const malformedPayloads: Record<string, unknown>[] = [
        {},
        { not_result: 1 },
      ];
      let served = 0;
      const { responder, client, seedClient } = await dialWithResponder({
        clientTransport: (clientEnd) => ({
          send: async (envelope) => {
            const doc = decodeWire(envelope);
            if (isConnectInvokeResponse(doc)) {
              const payload = malformedPayloads[served];
              if (payload !== undefined) {
                served += 1;
                await clientEnd.send(
                  encodeEnvelope(
                    await authenticateInvokeResponse(seedClient, {
                      session_id: doc.session_id,
                      sequence: doc.sequence,
                      request_id: doc.request_id,
                      payload,
                    }),
                  ),
                );
                return;
              }
            }
            await clientEnd.send(envelope);
          },
          recv: () => clientEnd.recv(),
          close: () => clientEnd.close?.(),
        }),
      });
      try {
        client.registerToolHandler("tools.math.add", addHandler([]));
        for (const malformed of malformedPayloads) {
          const result = await responder.invokeTool("tools.math.add", {
            a: 1,
            b: 2,
          });
          expect(result).toEqual({
            ok: false,
            code: SpokeRejectCode.INTERNAL_ERROR,
            message: expect.stringContaining("payload decode failed"),
            details: { kind: "transport" },
          });
          // note: the malformed payload fails only this waiter — the session
          // stays usable for the next reverse invoke.
          expect(responder.state).toBe("Established");
          expect(client.state).toBe("Established");
        }
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "times out the waiter without closing the session (waiter-only timeout)",
    async () => {
      // why: the dialer's handler never resolves — the request DID hit the wire
      // (outbound sequence transmitted), so the waiter times out but the
      // session stays usable on both ends (no poison-close).
      const { responder, client } = await dialWithResponder({
        responderTimeoutMs: 100,
      });
      try {
        client.registerToolHandler(
          "tools.echo.boom",
          // why: `Promise.withResolvers` needs the ES2024 lib target, and a
          // never-settling promise has no resolvers to call anyway.
          () => new Promise<never>(() => {}),
        );
        const timedOut = await responder.invokeTool("tools.echo.boom", {});
        expect(timedOut).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        client.registerToolHandler("tools.math.add", async (args) =>
          spokeOk({ sum: (args.a as number) + (args.b as number) }),
        );
        const retry = await responder.invokeTool("tools.math.add", {
          a: 40,
          b: 2,
        });
        expect(retry).toEqual({ ok: true, value: { sum: 42 } });
        expect(responder.state).toBe("Established");
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "closes the session when a timed-out queued reverse invoke's send is skipped (poison-close mirror)",
    async () => {
      // why: wire-level injector on the RESPONDER end — the FIRST outbound
      // reverse invoke's send is delayed on the wire, and the second reverse
      // invoke's send is serialized behind it (send tail) and times out
      // while waiting, before its send ever starts. When the first send
      // finally completes, the queued send is skipped: its waiter already
      // settled, so transmitting it late would be a duplicate dispatch on
      // the dialer. The skip must instead close the session (the allocated
      // outbound sequence never hit the wire, so the dialer's inbound gate
      // would be stuck at it), mirroring the adapter's `#invokeOp`.
      const sentSequences: number[] = [];
      let firstSendDelayed = false;
      const { responder, client } = await dialWithResponder({
        responderTimeoutMs: 100,
        responderTransport: (serverEnd) => ({
          send: async (envelope) => {
            const doc = decodeWire(envelope);
            if ("op" in doc) {
              if (!firstSendDelayed) {
                firstSendDelayed = true;
                // note: real-clock delay is deliberate (integration exception) —
                // the race under test IS between the responder's invoke
                // timeout timer and a slow transport send, with both peers
                // on real timers; fake timers cannot drive the loopback
                // transport or WebCrypto signing. The 200ms wire delay sits
                // far outside the 100ms invoke timeout, so the ordering is
                // deterministic under load.
                await delay(200);
              }
              sentSequences.push(doc.sequence as number);
            }
            await serverEnd.send(envelope);
          },
          recv: () => serverEnd.recv(),
          close: () => serverEnd.close?.(),
        }),
      });
      try {
        client.registerToolHandler("tools.math.add", async (args) =>
          spokeOk({ sum: (args.a as number) + (args.b as number) }),
        );
        const [first, second] = await Promise.all([
          responder.invokeTool("tools.math.add", { a: 1, b: 2 }),
          responder.invokeTool("tools.math.add", { a: 3, b: 4 }),
        ]);
        expect(first).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        expect(second).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        await untilState(responder, "Closed", "responder");
        expect(sentSequences).toEqual([0]);
        const after = await responder.invokeTool("tools.math.add", {
          a: 5,
          b: 6,
        });
        expect(after).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("connect session is not established"),
          details: { kind: "session_closed" },
        });
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );
});

describe("connectResponder forward tool serving", () => {
  it(
    "serves a forward invokeTool from the dialer through a registered handler",
    async () => {
      const responderCalls: { args: Record<string, unknown> }[] = [];
      const { responder, client } = await dialWithResponder();
      try {
        responder.registerToolHandler("tools.math.add", addHandler(responderCalls));
        const result = await client.invokeTool("tools.math.add", {
          a: 21,
          b: 21,
        });
        expect(result).toEqual({ ok: true, value: { sum: 42 } });
        expect(responderCalls).toEqual([{ args: { a: 21, b: 21 } }]);
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "keeps the error path for a tools.* invoke whose any-JSON payload is null",
    async () => {
      // why: regression guard (pre-fix semantics restored) — the wire `payload` is
      // any-JSON, so a null payload reaches the tool-argument read with no
      // structural gate on the serving side. It must fail into the serving
      // error branch, never be defaulted to `{}` and handed to the handler.
      const responderCalls: { args: Record<string, unknown> }[] = [];
      const { responder, clientEnd, seedClient, pubkeyResponder, peerIdResponder } =
        await startRawResponder();
      try {
        responder.registerToolHandler("tools.math.add", addHandler(responderCalls));
        const { session_id: sessionId } = await rawHandshake(clientEnd, {
          seed: seedClient,
          manifest: toolManifest("test-client"),
          pubkeyResponder,
          peerIdResponder,
        });
        const nullPayload = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 0,
          request_id: "null-payload-tool-invoke",
          op: "tools.math.add",
          payload: null,
        });
        await clientEnd.send(encodeEnvelope(nullPayload));
        const response = decodeEnvelope(await clientEnd.recv()) as {
          error: { code: string; message: string };
          payload?: unknown;
        };
        expect(response.error.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
        expect(response.error.message).toContain("invoke failed");
        expect(response).not.toHaveProperty("payload");
        expect(responderCalls).toEqual([]);
        expect(responder.state).toBe("Established");
      } finally {
        responder.close();
      }
    },
    15000,
  );

  it(
    "answers the error branch when a served handler throws, without loop damage",
    async () => {
      const boomCalls: string[] = [];
      const { responder, client } = await dialWithResponder();
      try {
        responder.registerToolHandler("tools.echo.boom", async () => {
          boomCalls.push("boom");
          throw new Error("provider exploded");
        });
        const thrown = await client.invokeTool("tools.echo.boom", {});
        expect(thrown).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: "provider exploded",
        });
        expect(boomCalls).toEqual(["boom"]);
        // note: loop damage check — the responder's serve loop survived, so a
        // different forward invoke for a healthy handler still succeeds.
        responder.registerToolHandler("tools.math.add", async (args) =>
          spokeOk({ sum: (args.a as number) + (args.b as number) }),
        );
        const healthy = await client.invokeTool("tools.math.add", { a: 40, b: 2 });
        expect(healthy).toEqual({ ok: true, value: { sum: 42 } });
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "rejects registerToolHandler for a non-tool capability id (grammar gate)",
    async () => {
      const { responder, client } = await dialWithResponder();
      try {
        expect(() =>
          responder.registerToolHandler("spoke-baseline", async () =>
            spokeOk({}),
          ),
        ).toThrow(/must start with "tools." prefix/);
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );
});

describe("connectResponder carried-over demo behaviors", () => {
  it(
    "still establishes on an empty capabilities intersection (snapshot fallback)",
    async () => {
      // why: disjoint capability sets — the negotiated intersection is empty, so
      // the responder's signed session snapshot must fall back to
      // `["spoke-baseline"]` (wire minItems 1). The dialer computes its own
      // intersection for gating, so the fallback has no authorization
      // impact; the dial must simply establish.
      const clientOnly: HostCapabilityManifest = {
        ...schemaConformantManifest(),
        host_id: "test-client",
        namespaces: ["toy_world"],
        capabilities: ["client-only-capability"],
      };
      const responderOnly: HostCapabilityManifest = {
        ...schemaConformantManifest(),
        host_id: "test-responder",
        namespaces: ["toy_world"],
        capabilities: ["responder-only-capability"],
      };
      const { responder, client } = await dialWithResponder({
        clientManifest: clientOnly,
        responderManifest: responderOnly,
      });
      try {
        expect(responder.state).toBe("Established");
        expect(client.state).toBe("Established");
        expect(responder.sessionId).toBe(client.sessionId);
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "closes the connection on an unparseable inbound frame",
    async () => {
      const { responder, client, pair } = await dialWithResponder();
      try {
        // why: a frame that fails JSON decode is a protocol violation, so the
        // responder's serve loop must actually close the transport (the
        // carried-over demo behavior); a bare return would leave the
        // client's established session hanging on its next invoke.
        await pair.client.send(new TextEncoder().encode("not json {{{"));
        await untilState(responder, "Closed", "responder");
        await untilState(client, "Closed", "client");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );
});

describe("connectResponder per-invoke gate (peek → verify → advance)", () => {
  it(
    "rejects a sequence-gap invoke with invalid_sequence and does NOT advance the counter",
    async () => {
      const { responder, clientEnd, seedClient, pubkeyResponder, peerIdResponder } =
        await startRawResponder();
      try {
        const { session_id: sessionId } = await rawHandshake(clientEnd, {
          seed: seedClient,
          manifest: toolManifest("test-client"),
          pubkeyResponder,
          peerIdResponder,
        });
        const gap = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 5,
          request_id: "seq-gap",
          op: "port.knowledge.get",
          payload: { entry_id: MIRA_ENTRY_ID },
        });
        await clientEnd.send(encodeEnvelope(gap));
        const rejection = decodeEnvelope(await clientEnd.recv()) as {
          error: { code: string };
        };
        expect(rejection.error.code).toBe("invalid_sequence");

        const valid = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 0,
          request_id: "valid-after-gap",
          op: "port.knowledge.get",
          payload: { entry_id: MIRA_ENTRY_ID },
        });
        await clientEnd.send(encodeEnvelope(valid));
        const okResponse = decodeEnvelope(await clientEnd.recv()) as {
          payload: { entry_id: string };
        };
        expect(okResponse.payload.entry_id).toBe(MIRA_ENTRY_ID);
      } finally {
        responder.close();
      }
    },
    15000,
  );

  it(
    "answers invalid_sequence for a present but non-numeric sequence (deny observability)",
    async () => {
      const { responder, clientEnd, seedClient, pubkeyResponder, peerIdResponder } =
        await startRawResponder();
      try {
        const { session_id: sessionId } = await rawHandshake(clientEnd, {
          seed: seedClient,
          manifest: toolManifest("test-client"),
          pubkeyResponder,
          peerIdResponder,
        });
        // note: a wire request whose `sequence` is present but is a STRING — the
        // strict `InboundSequence.peek` throws → `invalid_sequence` deny
        // (Rust mirrors this deny at the gate's sequence extraction: the
        // normative deny-observability parity, not a silent `Stray`).
        const malformed = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: "5" as unknown as number,
          request_id: "non-numeric-seq",
          op: "port.knowledge.get",
          payload: { entry_id: MIRA_ENTRY_ID },
        });
        await clientEnd.send(encodeEnvelope(malformed));
        const rejection = decodeEnvelope(await clientEnd.recv()) as {
          error: { code: string };
        };
        expect(rejection.error.code).toBe("invalid_sequence");

        const valid = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 0,
          request_id: "valid-after-non-numeric",
          op: "port.knowledge.get",
          payload: { entry_id: MIRA_ENTRY_ID },
        });
        await clientEnd.send(encodeEnvelope(valid));
        const okResponse = decodeEnvelope(await clientEnd.recv()) as {
          payload: { entry_id: string };
        };
        expect(okResponse.payload.entry_id).toBe(MIRA_ENTRY_ID);
      } finally {
        responder.close();
      }
    },
    15000,
  );

  it(
    "rejects a tampered invoke with auth_failed and does NOT advance the counter",
    async () => {
      const { responder, clientEnd, seedClient, pubkeyResponder, peerIdResponder } =
        await startRawResponder();
      try {
        const { session_id: sessionId } = await rawHandshake(clientEnd, {
          seed: seedClient,
          manifest: toolManifest("test-client"),
          pubkeyResponder,
          peerIdResponder,
        });
        // note: wire-level tamper — mutate the payload AFTER signing, so the
        // envelope-auth verify must fail BEFORE advance (auth-before-
        // advance), answering auth_failed with the locked details.kind.
        const tampered = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 0,
          request_id: "tampered",
          op: "port.knowledge.get",
          payload: { entry_id: MIRA_ENTRY_ID },
        });
        (tampered.payload as Record<string, unknown>).tampered = true;
        await clientEnd.send(encodeEnvelope(tampered));
        const rejection = decodeEnvelope(await clientEnd.recv()) as {
          error: { code: string; details?: Record<string, unknown> };
        };
        expect(rejection.error.code).toBe("auth_failed");
        expect(rejection.error.details?.kind).toBe("envelope_auth_invalid");

        // note: auth-before-advance — the inbound counter is UNCHANGED, so the
        // same sequence re-issued with a valid signature succeeds.
        const retry = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 0,
          request_id: "valid-after-tamper",
          op: "port.knowledge.get",
          payload: { entry_id: MIRA_ENTRY_ID },
        });
        await clientEnd.send(encodeEnvelope(retry));
        const okResponse = decodeEnvelope(await clientEnd.recv()) as {
          payload: { entry_id: string };
        };
        expect(okResponse.payload.entry_id).toBe(MIRA_ENTRY_ID);
      } finally {
        responder.close();
      }
    },
    15000,
  );
});

const LOADER_CANARY = "LOADER_ONLY_CANARY_ke_remote_ts";

function manifestWithCaps(
  hostId: string,
  capabilities: readonly string[],
): HostCapabilityManifest {
  const base = schemaConformantManifest();
  // why: every fixture host here serves the toy ports, including the knowledge
  // face, so `spoke-knowledge` is always negotiated explicitly.
  const caps = [
    ...new Set([...base.capabilities, "spoke-knowledge", ...capabilities]),
  ] as [string, ...string[]];
  return { ...base, host_id: hostId, capabilities: caps };
}

function sampleExtractRequest(runId = "run-ke-remote-1"): ExtractRequest {
  return {
    run_id: runId,
    sources: [
      {
        schema_version: 1,
        source_id: "src-ke-remote",
        extensions: {},
      },
    ],
  };
}

function extractionPortsWithCanary(): ExtractionPort {
  return {
    loadExtractionInput: async () => spokeOk({ secret: LOADER_CANARY }),
  };
}

function toyBaselinePorts(): BaselinePorts & Partial<RemoteExtractService> {
  return asBaselineOnly(new ToyWorldAdapter()) as BaselinePorts &
    Partial<RemoteExtractService>;
}

describe("ke remote", () => {
  it(
    "extract round-trip echoes run_id, keeps candidates provisional, and omits loader canary from wire",
    async () => {
      const wireRequests: string[] = [];
      const wireResponses: string[] = [];
      const ports = toyBaselinePorts();
      const extract = (request: ExtractRequest) =>
        orchestrateExtract(extractionPortsWithCanary(), request, async () =>
          spokeOk({
            candidates: [
              {
                schema_version: 1,
                entry_id: "cand-1",
                entry_type: "character",
                canonical_name: "Cand",
                status: "provisional",
                body: { summary: "x" },
                extensions: {},
              },
            ],
          }),
        );
      ports.extract = extract;
      const { client, responder, pair } = await dialWithResponder({
        clientManifest: manifestWithCaps("client-extract", [
          CAPABILITY_KE_EXTRACTION,
        ]),
        responderManifest: manifestWithCaps("responder-extract", [
          CAPABILITY_KE_EXTRACTION,
          "input-source",
        ]),
        ports,
        clientTransport: (transport) => ({
          send: async (bytes) => {
            wireRequests.push(new TextDecoder().decode(bytes));
            return transport.send(bytes);
          },
          recv: async () => {
            const bytes = await transport.recv();
            wireResponses.push(new TextDecoder().decode(bytes));
            return bytes;
          },
          close: () => {
            transport.close?.();
          },
        }),
      });
      try {
        const result = await client.extract(sampleExtractRequest());
        expect(result.ok).toBe(true);
        if (!result.ok) return;
        if (!("run" in result.value)) {
          throw new Error("expected extract success branch");
        }
        expect(result.value.run.run_id).toBe("run-ke-remote-1");
        expect(result.value.candidates).toHaveLength(1);
        expect(result.value.candidates[0]?.status).toBe("provisional");
        const joined = wireRequests.join("\n");
        expect(joined).not.toContain(LOADER_CANARY);
        expect(joined).toContain("run-ke-remote-1");
        // why: both wire directions — the response bytes must carry the assembled
        // batch (positive control) and no loader value (F1: no content
        // channel; the request direction alone would leave the response
        // unproven).
        const joinedResponses = wireResponses.join("\n");
        expect(joinedResponses).toContain("run-ke-remote-1");
        expect(joinedResponses).not.toContain(LOADER_CANARY);
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it("denies extract when ke-extraction is missing from negotiated capabilities", async () => {
    const ports = toyBaselinePorts();
    ports.extract = async () =>
      spokeOk({ candidates: [], run: { run_id: "x" } });
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-no-extract", []),
      responderManifest: manifestWithCaps("responder-extract-ad", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      ports,
    });
    try {
      const result = await client.extract(sampleExtractRequest());
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details?.wire_code).toBe("op_unsupported");
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("probe-denies extract when capability is negotiated but extract service is absent", async () => {
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-extract-probe", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      responderManifest: manifestWithCaps("responder-extract-probe", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      ports: toyBaselinePorts(),
    });
    try {
      const result = await client.extract(sampleExtractRequest());
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details?.wire_code).toBe("op_unsupported");
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("probe-denies malformed extract payload when extract service is absent", async () => {
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-extract-probe-malformed", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      responderManifest: manifestWithCaps("responder-extract-probe-malformed", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      ports: toyBaselinePorts(),
    });
    try {
      const result = await client.extract({} as unknown as ExtractRequest);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details?.wire_code).toBe("op_unsupported");
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("maps service application reject separately from probe-deny", async () => {
    const ports = toyBaselinePorts();
    ports.extract = async () =>
      spokeReject(SpokeRejectCode.CAPABILITY_PORT_MISSING, "declined", {
        capability: "ke-extraction",
      });
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-extract-app", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      responderManifest: manifestWithCaps("responder-extract-app", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      ports,
    });
    try {
      const result = await client.extract(sampleExtractRequest());
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details?.wire_code).toBeUndefined();
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("normalizes an injected ExtractResponse error branch through the application reject path", async () => {
    const ports = toyBaselinePorts();
    ports.extract = async () =>
      spokeOk({
        error: {
          code: SpokeRejectCode.INVALID_INPUT,
          message: "bad request",
          extensions: {},
        },
      });
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-extract-err", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      responderManifest: manifestWithCaps("responder-extract-err", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      ports,
    });
    try {
      const result = await client.extract(sampleExtractRequest());
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("allows listKnowledgeEntries without viewpoint when spoke-knowledge is negotiated", async () => {
    const scope: Scope = { scope_id: "s1" };
    const calls: Scope[] = [];
    const ports = toyBaselinePorts();
    const original = ports.listKnowledgeEntries.bind(ports);
    ports.listKnowledgeEntries = async (s) => {
      calls.push(s);
      return original(s);
    };
    const { client, responder, pair } = await dialWithResponder({
      ports,
      admittedScope: { scope_id: "s1" },
    });
    try {
      const result = await client.listKnowledgeEntries(scope);
      expect(result.ok).toBe(true);
      expect(calls).toHaveLength(1);
      expect(calls[0]).toEqual(scope);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("requires ke-ownership for non-empty viewpoint and preserves scope fields", async () => {
    const scope: Scope = {
      scope_id: "s-own",
      viewpoint: "holder-a",
      extensions: { x: { flag: true } },
    };
    const calls: Scope[] = [];
    const ports = toyBaselinePorts();
    const original = ports.listKnowledgeEntries.bind(ports);
    ports.listKnowledgeEntries = async (s) => {
      calls.push(s);
      return original(s);
    };
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-own", [CAPABILITY_KE_OWNERSHIP]),
      responderManifest: manifestWithCaps("responder-own", [
        CAPABILITY_KE_OWNERSHIP,
      ]),
      ports,
      admittedScope: { scope_id: "s-own", viewpoint: "holder-a" },
    });
    try {
      const result = await client.listKnowledgeEntries(scope);
      expect(result.ok).toBe(true);
      expect(calls[0]?.viewpoint).toBe("holder-a");
      expect(calls[0]?.extensions).toEqual({ x: { flag: true } });
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("denies viewpoint-bearing list when ke-ownership is not negotiated", async () => {
    const scope: Scope = {
      scope_id: "s-deny",
      viewpoint: "holder-a",
    };
    const { client, responder, pair } = await dialWithResponder({
      ports: toyBaselinePorts(),
    });
    try {
      const result = await client.listKnowledgeEntries(scope);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details?.wire_code).toBe("op_unsupported");
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("serves a successful empty candidate batch", async () => {
    const ports = toyBaselinePorts();
    ports.extract = (request: ExtractRequest) =>
      orchestrateExtract(extractionPortsWithCanary(), request, async () =>
        spokeOk({ candidates: [] }),
      );
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-extract-empty", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      responderManifest: manifestWithCaps("responder-extract-empty", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      ports,
    });
    try {
      const result = await client.extract(sampleExtractRequest("run-empty"));
      expect(result.ok).toBe(true);
      if (!result.ok) return;
      if (!("run" in result.value)) {
        throw new Error("expected extract success branch");
      }
      expect(result.value.run.run_id).toBe("run-empty");
      expect(result.value.candidates).toEqual([]);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("rejects malformed declared scope payloads before the provider", () => {
    const op = "port.scope.list_knowledge_entries";
    const malformedPayloads: Record<string, unknown>[] = [
      { scope: { scope_id: "s1", viewpoint: "" } },
      { scope: { scope_id: "s1", viewpoint: null } },
      { scope: { scope_id: "s1", viewpoint: 3 } },
      { scope: { scope_id: "s1", viewpoint: [] } },
      { scope: { scope_id: "s1", viewpoint: {} } },
      { scope: { viewpoint: "holder-a" } },
      { scope: { scope_id: "s1", viewpoint: "holder-a", unknown_field: 1 } },
      {
        scope: {
          scope_id: "s1",
          viewpoint: "holder-a",
          entry_ids: "not-an-array",
        },
      },
      {},
      { scope: "not-an-object" },
    ];
    for (const payload of malformedPayloads) {
      const reject = validateScopeOpPayload(op, payload);
      expect(reject).not.toBeNull();
      expect(reject?.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
    expect(
      validateScopeOpPayload(op, { scope: { scope_id: "s1" } }),
    ).toBeNull();
    expect(
      validateScopeOpPayload(op, {
        scope: { scope_id: "s1", viewpoint: "holder-a" },
      }),
    ).toBeNull();
  });

  it("rejects schema-invalid declared scope with valid viewpoint before provider call", async () => {
    let called = 0;
    const ports = toyBaselinePorts();
    ports.listKnowledgeEntries = async () => {
      called += 1;
      return spokeOk([]);
    };
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-scope-schema", [
        CAPABILITY_KE_OWNERSHIP,
      ]),
      responderManifest: manifestWithCaps("responder-scope-schema", [
        CAPABILITY_KE_OWNERSHIP,
      ]),
      ports,
      admittedScope: { scope_id: "s1", viewpoint: "holder-a" },
    });
    try {
      const malformedScopes = [
        { viewpoint: "holder-a" },
        { scope_id: "s1", viewpoint: "holder-a", unknown_field: 1 },
        {
          scope_id: "s1",
          viewpoint: "holder-a",
          entry_ids: "not-an-array",
        },
      ] as unknown as Scope[];
      for (const badScope of malformedScopes) {
        const result = await client.listKnowledgeEntries(badScope);
        expect(result.ok).toBe(false);
        if (result.ok) return;
        expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      }
      expect(called).toBe(0);

      const validScope: Scope = {
        scope_id: "s1",
        viewpoint: "holder-a",
      };
      const okResult = await client.listKnowledgeEntries(validScope);
      expect(okResult.ok).toBe(true);
      expect(called).toBe(1);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("rejects malformed viewpoint with INVALID_INPUT before provider call", async () => {
    let called = false;
    const ports = toyBaselinePorts();
    ports.listKnowledgeEntries = async () => {
      called = true;
      return spokeOk([]);
    };
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-malformed", [
        CAPABILITY_KE_OWNERSHIP,
      ]),
      responderManifest: manifestWithCaps("responder-malformed", [
        CAPABILITY_KE_OWNERSHIP,
      ]),
      ports,
    });
    try {
      const badScope = {
        scope_id: "bad",
        viewpoint: "",
      } as Scope;
      const result = await client.listKnowledgeEntries(badScope);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(called).toBe(false);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });



  it("rejects malformed declared scope with non-empty viewpoint as INVALID_INPUT before ownership gate when ke-ownership is not negotiated", async () => {
    let called = false;
    const ports = toyBaselinePorts();
    ports.listKnowledgeEntries = async () => {
      called = true;
      return spokeOk([]);
    };
    const { client, responder, pair } = await dialWithResponder({ ports });
    try {
      const badScope = {
        scope_id: "s1",
        viewpoint: "holder-a",
        unknown_field: 1,
      } as unknown as Scope;
      const result = await client.listKnowledgeEntries(badScope);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details?.wire_code).toBeUndefined();
      expect(called).toBe(false);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("rejects malformed declared scope with INVALID_INPUT when ke-ownership is not negotiated", async () => {
    let called = false;
    const ports = toyBaselinePorts();
    ports.listKnowledgeEntries = async () => {
      called = true;
      return spokeOk([]);
    };
    const { client, responder, pair } = await dialWithResponder({ ports });
    try {
      const badScope = {
        scope_id: "bad",
        viewpoint: null,
      } as unknown as Scope;
      const result = await client.listKnowledgeEntries(badScope);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details?.wire_code).toBeUndefined();
      expect(called).toBe(false);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("rejects malformed extract request payloads before the provider", () => {
    const malformedPayloads: unknown[] = [
      null,
      { run_id: "", sources: [{ schema_version: 1, source_id: "s", extensions: {} }] },
      { run_id: "run-1", sources: [] },
      {
        run_id: "run-1",
        sources: [{ schema_version: 0, source_id: "s", extensions: {} }],
      },
      {
        run_id: "run-1",
        sources: [{ schema_version: 1, source_id: "s" }],
      },
      {
        run_id: "run-1",
        sources: [{ schema_version: 1, source_id: "s", extensions: {} }],
        reuqest_id: "x",
      },
      {
        run_id: "run-1",
        sources: [
          {
            schema_version: 1,
            source_id: "s",
            extensions: {},
            unknown_anchor_key: 1,
          },
        ],
      },
      {
        run_id: "run-1",
        sources: [
          {
            schema_version: 1,
            source_id: "s",
            extensions: {},
            span: "not-an-object",
          },
        ],
      },
      {
        run_id: "run-1",
        sources: [
          {
            schema_version: 1,
            source_id: "s",
            extensions: {},
            span: { start: "bad", end: 4 },
          },
        ],
      },
    ];
    for (const payload of malformedPayloads) {
      const reject = validateExtractRequestPayload(payload);
      expect(reject).not.toBeNull();
      expect(reject?.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
    expect(
      validateExtractRequestPayload(sampleExtractRequest("run-valid")),
    ).toBeNull();
  });

  it("distinguishes missing scope from present-but-non-object scope in reject messages", () => {
    const op = "port.scope.list_knowledge_entries";
    const missing = validateScopeOpPayload(op, {});
    expect(missing).not.toBeNull();
    expect(missing?.message).toContain("missing scope");

    const nonObject = validateScopeOpPayload(op, { scope: "s1" });
    expect(nonObject).not.toBeNull();
    expect(nonObject?.code).toBe(SpokeRejectCode.INVALID_INPUT);
    expect(nonObject?.message).toContain("scope must be an object");
    expect(nonObject?.message).not.toContain("missing scope");
  });

  it("rejects extract payloads with unknown top-level keys before the provider", async () => {
    let called = false;
    const ports = toyBaselinePorts();
    ports.extract = async () => {
      called = true;
      return spokeOk({ candidates: [], run: { run_id: "x" } });
    };
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-extract-junk-key", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      responderManifest: manifestWithCaps("responder-extract-junk-key", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      ports,
    });
    try {
      const badRequest = {
        run_id: "run-1",
        sources: [{ schema_version: 1, source_id: "s", extensions: {} }],
        reuqest_id: "x",
      } as unknown as ExtractRequest;
      const result = await client.extract(badRequest);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.message).toContain("unknown property");
      expect(called).toBe(false);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("rejects malformed extract request with INVALID_INPUT before provider call", async () => {
    let called = false;
    const ports = toyBaselinePorts();
    ports.extract = async () => {
      called = true;
      return spokeOk({ candidates: [], run: { run_id: "x" } });
    };
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-extract-malformed", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      responderManifest: manifestWithCaps("responder-extract-malformed", [
        CAPABILITY_KE_EXTRACTION,
      ]),
      ports,
    });
    try {
      const badRequest = {
        run_id: "",
        sources: [],
      } as unknown as ExtractRequest;
      const result = await client.extract(badRequest);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(called).toBe(false);
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("rejects malformed fork scope with INVALID_INPUT before the missing fork-face probe", async () => {
    const { client, responder, pair } = await dialWithResponder({
      clientManifest: manifestWithCaps("client-fork-malformed", ["l5-fork"]),
      responderManifest: manifestWithCaps("responder-fork-malformed", [
        "l5-fork",
      ]),
      ports: toyBaselinePorts(),
    });
    try {
      const badScope = {
        scope_id: "pkt-scope",
        fork_id: "fork-1",
        viewpoint: null,
      } as unknown as Scope & { fork_id: ForkId };
      const result = await client.listForkTimelineEvents(badScope);
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details?.wire_code).toBeUndefined();
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });
});

/** note: a promise plus the externally driven settle handles for a parked provider. */
function parkingGate(): {
  promise: Promise<void>;
  release: () => void;
  fail: (error: unknown) => void;
} {
  let release!: () => void;
  let fail!: (error: unknown) => void;
  const promise = new Promise<void>((resolve, reject) => {
    // why: the project's TS lib target predates `Promise.withResolvers` (ES2024).
    release = () => resolve();
    fail = (error) => reject(error);
  });
  return { promise, release, fail };
}

/**
 * note: provider double for the serve-budget cases: `listKnowledgeEntries`,
 * `compute` and `extract` record entry and park until the test releases (or
 * fails) the gate, so a witness can prove the responder's own local serve
 * budget bounded the wait while the dialer's budget stayed far longer. Every
 * other method delegates; the optional/fork faces stay absent (baseline-only
 * projection).
 */
interface ParkedProvider {
  ports: BaselinePorts & Partial<RemoteExtractService>;
  calls: string[];
  release: () => void;
  fail: (error: unknown) => void;
}

function parkedProvider(): ParkedProvider {
  const full = ToyWorldAdapter.withCommittedFixtures();
  const gate = parkingGate();
  const calls: string[] = [];
  const ports = {
    ...asBaselineOnly(full),
    listKnowledgeEntries: async (scope: Scope) => {
      calls.push("listKnowledgeEntries");
      await gate.promise;
      return full.listKnowledgeEntries(scope);
    },
    compute: async (request: ComputeRequest) => {
      calls.push("compute");
      await gate.promise;
      return full.compute(request);
    },
    extract: async (request: ExtractRequest) => {
      calls.push("extract");
      await gate.promise;
      return spokeOk({ candidates: [], run: { run_id: request.run_id } });
    },
  };
  return { ports, calls, release: gate.release, fail: gate.fail };
}

/**
 * note: inbound wire capture for the serve-wait witnesses: every frame the dialer
 * receives, decoded, so the served response can be asserted instead of an
 * elapsed-time proxy.
 */
function wireCapture(
  frames: Record<string, unknown>[],
): (transport: Transport) => Transport {
  return (transport) => ({
    send: (bytes) => transport.send(bytes),
    recv: async () => {
      const bytes = await transport.recv();
      frames.push(decodeWire(bytes));
      return bytes;
    },
    close: () => {
      transport.close?.();
    },
  });
}

function framesWith(
  frames: Record<string, unknown>[],
  field: string,
): Record<string, unknown>[] {
  return frames.filter((frame) => frame[field] !== undefined);
}

/**
 * note: outbound capture for the close-invalidation witnesses: every frame the
 * RESPONDER attempts to send, decoded. The attempt is the observable — the
 * loopback transport rejects a send once the connection is closed, so a
 * delivered frame can no longer be captured on the other end.
 */
function sentCapture(
  frames: Record<string, unknown>[],
): (transport: Transport) => Transport {
  return (transport) => ({
    send: (bytes) => {
      frames.push(decodeWire(bytes));
      return transport.send(bytes);
    },
    recv: () => transport.recv(),
    close: () => {
      transport.close?.();
    },
  });
}

/** note: bounded poll (5s) until the parked provider has recorded `method` entry. */
async function untilParked(
  parked: ParkedProvider,
  method: string,
): Promise<void> {
  const deadline = Date.now() + 5000;
  while (!parked.calls.includes(method)) {
    if (Date.now() >= deadline) {
      throw new Error(
        `${method} never reached the parked provider (calls ${parked.calls.join(",")})`,
      );
    }
    await delay(5);
  }
}

/** note: bounded poll (500ms) until every in-flight `#withServeBudget` wait has settled. */
async function untilServeBudgetDrained(
  responder: ConnectResponder,
): Promise<void> {
  const deadline = Date.now() + 500;
  while (testActiveServeBudgetWaits(responder) > 0) {
    if (Date.now() >= deadline) {
      throw new Error(
        `serve budget wait still in flight (${testActiveServeBudgetWaits(responder)} active)`,
      );
    }
    await delay(5);
  }
}

function expectServedTimeout(
  frame: Record<string, unknown>,
  budgetMs: number,
): void {
  expect(frame.error).toEqual({
    code: SpokeRejectCode.INTERNAL_ERROR,
    message: expect.stringContaining(
      `serve wait exceeded the local budget of ${budgetMs}ms`,
    ),
    details: { kind: "timeout" },
    extensions: {},
  });
}

describe("serve timeout (local serve budget)", () => {
  it(
    "bounds a parked Scope port provider, answers one signed timeout and keeps the session usable",
    async () => {
      // why: the provider parks and the responder's own 25ms budget expires while
      // the dialer's budget is far longer (2000ms), so the observed reject can
      // only come from the serve-side bound.
      const parked = parkedProvider();
      const frames: Record<string, unknown>[] = [];
      const { responder, client, pair } = await dialWithResponder({
        ports: parked.ports,
        admittedScope: { scope_id: "serve-timeout-scope" },
        responderTimeoutMs: 25,
        clientTimeoutMs: 2000,
        clientTransport: wireCapture(frames),
      });
      try {
        const scope: Scope = { scope_id: "serve-timeout-scope" };
        const result = await client.listKnowledgeEntries(scope);
        expect(result.ok).toBe(false);
        if (result.ok) return;
        expect(result.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
        expect(result.details?.kind).toBe("timeout");
        expect(result.message).toContain(
          "serve wait exceeded the local budget of 25ms",
        );

        expect(parked.calls).toEqual(["listKnowledgeEntries"]);
        const errors = framesWith(frames, "error");
        expect(errors).toHaveLength(1);
        expectServedTimeout(errors[0]!, 25);

        // note: a late settlement is discarded — releasing the parked provider
        // after expiry produces no second response.
        const payloads = framesWith(frames, "payload").length;
        parked.release();
        await delay(50);
        expect(framesWith(frames, "error")).toHaveLength(1);
        expect(framesWith(frames, "payload")).toHaveLength(payloads);

        expect(responder.state).toBe("Established");
        expect(client.state).toBe("Established");
        const followUp = await client.getKnowledgeEntry(MIRA_ENTRY_ID);
        expect(followUp.ok).toBe(true);
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it(
    "bounds a parked optional-family provider",
    async () => {
      const parked = parkedProvider();
      const frames: Record<string, unknown>[] = [];
      const { responder, client, pair } = await dialWithResponder({
        ports: parked.ports,
        admittedScope: { scope_id: "serve-timeout-scope" },
        clientManifest: manifestWithCaps("client-serve-compute", [
          "l2-computable",
        ]),
        responderManifest: manifestWithCaps("responder-serve-compute", [
          "l2-computable",
        ]),
        responderTimeoutMs: 25,
        clientTimeoutMs: 2000,
        clientTransport: wireCapture(frames),
      });
      try {
        const request: ComputeRequest = {
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          computable: { tide_level: 2.5, cargo_tons: 37 },
          settle: true,
        };
        const result = await client.compute(request);
        expect(result.ok).toBe(false);
        if (result.ok) return;
        expect(result.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
        expect(result.details?.kind).toBe("timeout");
        expect(result.message).toContain(
          "serve wait exceeded the local budget of 25ms",
        );

        expect(parked.calls).toEqual(["compute"]);
        const errors = framesWith(frames, "error");
        expect(errors).toHaveLength(1);
        expectServedTimeout(errors[0]!, 25);
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it(
    "bounds a parked extract service and consumes the late rejection",
    async () => {
      // note: the whole extraction service (loader + extractor) is one bounded
      // wait, and a late rejection is consumed — no unhandled rejection, no
      // second response.
      const parked = parkedProvider();
      const frames: Record<string, unknown>[] = [];
      const { responder, client, pair } = await dialWithResponder({
        ports: parked.ports,
        admittedScope: { scope_id: "serve-timeout-scope" },
        clientManifest: manifestWithCaps("client-serve-extract", [
          CAPABILITY_KE_EXTRACTION,
        ]),
        responderManifest: manifestWithCaps("responder-serve-extract", [
          CAPABILITY_KE_EXTRACTION,
        ]),
        responderTimeoutMs: 25,
        clientTimeoutMs: 2000,
        clientTransport: wireCapture(frames),
      });
      try {
        const result = await client.extract(
          sampleExtractRequest("run-serve-timeout"),
        );
        expect(result.ok).toBe(false);
        if (result.ok) return;
        expect(result.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
        expect(result.details?.kind).toBe("timeout");
        expect(result.message).toContain(
          "serve wait exceeded the local budget of 25ms",
        );

        expect(parked.calls).toEqual(["extract"]);
        const errors = framesWith(frames, "error");
        expect(errors).toHaveLength(1);
        expectServedTimeout(errors[0]!, 25);

        const payloads = framesWith(frames, "payload").length;
        parked.fail(new Error("late provider rejection"));
        await delay(50);
        expect(framesWith(frames, "error")).toHaveLength(1);
        expect(framesWith(frames, "payload")).toHaveLength(payloads);
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it(
    "zero budget answers the signed timeout before any provider call",
    async () => {
      const parked = parkedProvider();
      const frames: Record<string, unknown>[] = [];
      const { responder, client, pair } = await dialWithResponder({
        ports: parked.ports,
        admittedScope: { scope_id: "serve-timeout-scope" },
        responderTimeoutMs: 0,
        clientTimeoutMs: 2000,
        clientTransport: wireCapture(frames),
      });
      try {
        const result = await client.listKnowledgeEntries({
          scope_id: "serve-timeout-scope",
        });
        expect(result.ok).toBe(false);
        if (result.ok) return;
        expect(result.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
        expect(result.details?.kind).toBe("timeout");
        expect(result.message).toContain(
          "serve wait exceeded the local budget of 0ms",
        );

        expect(parked.calls).toEqual([]);
        const errors = framesWith(frames, "error");
        expect(errors).toHaveLength(1);
        expectServedTimeout(errors[0]!, 0);

        // note: the bounded wait never closes the session — a further invoke on
        // the same session is answered on the wire again.
        expect(responder.state).toBe("Established");
        const again = await client.getKnowledgeEntry(MIRA_ENTRY_ID);
        expect(again.ok).toBe(false);
        if (again.ok) return;
        expect(again.details?.kind).toBe("timeout");
        expect(parked.calls).toEqual([]);
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it(
    "zero budget keeps gate, probe and ownership denials ahead of the bound",
    async () => {
      // note: `l5-fork` negotiated, `l2-computable` not — one dial covers the
      // capability deny, the optional-face probe deny and the payload-
      // dependent ownership deny, all of which run before the serve bound.
      const parked = parkedProvider();
      const { responder, client, pair } = await dialWithResponder({
        ports: parked.ports,
        admittedScope: { scope_id: "serve-timeout-scope" },
        clientManifest: manifestWithCaps("client-serve-deny", ["l5-fork"]),
        responderManifest: manifestWithCaps("responder-serve-deny", [
          "l5-fork",
        ]),
        responderTimeoutMs: 0,
        clientTimeoutMs: 2000,
      });
      try {
        const compute = await client.compute({
          session_id: "sess_tw_dawn_arrival",
          entry_id: "kb_tw_harbor",
          computable: { tide_level: 2.5 },
          settle: true,
        });
        expect(compute).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("not authorized"),
          details: { wire_code: "op_unsupported" },
        });

        const fork = await client.listForkTimelineEvents({
          scope_id: "serve-timeout-probe",
          fork_id: "fork_tw_storm_branch",
        });
        expect(fork).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining(
            "requires optional port method listForkTimelineEvents",
          ),
          details: { wire_code: "op_unsupported" },
        });

        const owned = await client.listKnowledgeEntries({
          scope_id: "serve-timeout-ownership",
          viewpoint: "holder-a",
        });
        expect(owned).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("requires capability ke-ownership"),
          details: { wire_code: "op_unsupported" },
        });

        expect(parked.calls).toEqual([]);
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it(
    "zero budget answers INVALID_INPUT for a malformed knowledge payload (Rust decode_knowledge_call parity)",
    async () => {
      // why: face split (QC W-001) closed by B2 — the TS face now mirrors the Rust
      // `decode_knowledge_call`, so a malformed `port.knowledge.get` payload is
      // decoded before the serve bound and keeps its `INVALID_INPUT`
      // reject at any budget, with zero provider calls (the provider never
      // starts).
      const baseline = asBaselineOnly(ToyWorldAdapter.withCommittedFixtures());
      const calls: string[] = [];
      const ports: BaselinePorts = {
        ...baseline,
        getKnowledgeEntry: async (entryId: string) => {
          calls.push(`getKnowledgeEntry:${String(entryId)}`);
          return baseline.getKnowledgeEntry(entryId);
        },
      };
      const { responder, clientEnd, seedClient, pubkeyResponder, peerIdResponder } =
        await startRawResponder({ ports, invokeTimeoutMs: 0 });
      try {
        const { session_id: sessionId } = await rawHandshake(clientEnd, {
          seed: seedClient,
          manifest: toolManifest("test-client"),
          pubkeyResponder,
          peerIdResponder,
        });
        // note: a non-string `entry_id` is malformed for the D4 row the Rust face
        // decodes before its bound.
        const malformed = await signInvokeRequest(seedClient, {
          session_id: sessionId,
          sequence: 0,
          request_id: "serve-timeout-malformed-baseline",
          op: "port.knowledge.get",
          payload: { entry_id: 7 },
        });
        await clientEnd.send(encodeEnvelope(malformed));
        const response = decodeEnvelope(await clientEnd.recv()) as {
          error?: {
            code?: string;
            message?: string;
            details?: Record<string, unknown>;
          };
        };
        expect(response.error?.code).toBe(SpokeRejectCode.INVALID_INPUT);
        expect(calls).toEqual([]);
        expect(responder.state).toBe("Established");
      } finally {
        responder.close();
      }
    },
    15000,
  );

  it(
    "does not bound a provider call that completes inside the budget",
    async () => {
      const { responder, client, pair } = await dialWithResponder({
        ports: asBaselineOnly(ToyWorldAdapter.withCommittedFixtures()),
        responderTimeoutMs: 25,
        clientTimeoutMs: 2000,
      });
      try {
        const result = await client.listKnowledgeEntries({
          scope_id: "toy-scope-001",
        });
        expect(result.ok).toBe(true);
        if (!result.ok) return;
        expect(
          result.value.some((entry) => entry.entry_id === MIRA_ENTRY_ID),
        ).toBe(true);
        expect(responder.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it(
    "answers nothing when the provider settles after the responder closed",
    async () => {
      // why: a close releases the session, so a provider that settles afterwards
      // must not answer a late success or a late error. The wide 200ms budget
      // keeps the parked provider's release well inside the wait, so this
      // witness covers the closed-state check rather than the timer.
      const parked = parkedProvider();
      const sent: Record<string, unknown>[] = [];
      const { responder, client, pair } = await dialWithResponder({
        ports: parked.ports,
        admittedScope: { scope_id: "serve-timeout-scope" },
        responderTimeoutMs: 200,
        clientTimeoutMs: 2000,
        responderTransport: sentCapture(sent),
      });
      try {
        const scope: Scope = { scope_id: "serve-timeout-scope" };
        // note: the close fails the dialer's waiter (the session is gone), so the
        // witness asserts the responder's own send attempts instead.
        void client.listKnowledgeEntries(scope).catch(() => undefined);
        await untilParked(parked, "listKnowledgeEntries");
        expect(parked.calls).toEqual(["listKnowledgeEntries"]);

        responder.close();
        const attemptedAtClose = sent.length;
        parked.release();
        await delay(50);

        expect(responder.state).toBe("Closed");
        expect(sent).toHaveLength(attemptedAtClose);
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it(
    "settles the in-flight serve budget wait when the responder closes",
    async () => {
      // why: close must settle `#withServeBudget`'s race, not only clear its timer —
      // a never-released provider leaves `pending` unresolved, and a disarm
      // that only clears the timer leaves `expired` unsettled too, so the
      // fire-and-forget dispatch would hang indefinitely.
      const parked = parkedProvider();
      const sent: Record<string, unknown>[] = [];
      const { responder, client, pair } = await dialWithResponder({
        ports: parked.ports,
        admittedScope: { scope_id: "serve-timeout-scope" },
        responderTimeoutMs: 25,
        clientTimeoutMs: 2000,
        responderTransport: sentCapture(sent),
      });
      try {
        const scope: Scope = { scope_id: "serve-timeout-scope" };
        void client.listKnowledgeEntries(scope).catch(() => undefined);
        await untilParked(parked, "listKnowledgeEntries");
        expect(testActiveServeBudgetWaits(responder)).toBe(1);

        responder.close();
        await untilServeBudgetDrained(responder);
        const attemptedAtClose = sent.length;
        await delay(60);
        expect(sent).toHaveLength(attemptedAtClose);

        parked.release();
        await delay(50);
        expect(responder.state).toBe("Closed");
        expect(sent).toHaveLength(attemptedAtClose);
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );

  it(
    "disarms the serve budget when the responder closes mid-wait",
    async () => {
      // why: the armed serve timer must not outlive the close — past the 25ms
      // budget a surviving timer would have answered the signed timeout, so
      // the absence of any send attempt after the budget elapsed is the
      // disarm witness. A provider settling later is discarded too.
      const parked = parkedProvider();
      const sent: Record<string, unknown>[] = [];
      const { responder, client, pair } = await dialWithResponder({
        ports: parked.ports,
        admittedScope: { scope_id: "serve-timeout-scope" },
        responderTimeoutMs: 25,
        clientTimeoutMs: 2000,
        responderTransport: sentCapture(sent),
      });
      try {
        const scope: Scope = { scope_id: "serve-timeout-scope" };
        void client.listKnowledgeEntries(scope).catch(() => undefined);
        await untilParked(parked, "listKnowledgeEntries");

        responder.close();
        const attemptedAtClose = sent.length;
        await delay(60);
        expect(sent).toHaveLength(attemptedAtClose);

        parked.release();
        await delay(50);
        expect(responder.state).toBe("Closed");
        expect(sent).toHaveLength(attemptedAtClose);
      } finally {
        client.close();
        responder.close();
        pair.client.close();
        pair.server.close();
      }
    },
    15000,
  );
});

function memoryManifest(hostId: string): HostCapabilityManifest {
  const base = toolManifest(hostId);
  return {
    ...base,
    capabilities: [
      ...base.capabilities,
      "spoke-memory",
      "spoke-memory-reflection",
      "spoke-memory-transfer",
    ],
  };
}

interface MemoryVector {
  op: string;
  method: string;
  request: Record<string, unknown>;
  response: Record<string, unknown>;
}

function memoryVectors(): MemoryVector[] {
  const fixture = JSON.parse(
    readFileSync(
      new URL("../fixtures/memory-invoke-golden.json", import.meta.url),
      "utf8",
    ),
  ) as { vectors: MemoryVector[] };
  return fixture.vectors;
}

/** note: ops exercised end-to-end through the adapter + responder seam. */
const MEMORY_VECTOR_OPS = [
  "memory.describe",
  "memory.capture",
  "memory.candidates.review",
  "memory.exports.get",
  "memory.imports.stage",
  "memory.receipts.get",
] as const;

describe("connectResponder memory.* seam (protocol §13)", () => {
  it("serves the representative golden vectors through the injected memory service", async () => {
    const vectors = memoryVectors().filter((vector) =>
      (MEMORY_VECTOR_OPS as readonly string[]).includes(vector.op),
    );
    expect(vectors).toHaveLength(MEMORY_VECTOR_OPS.length);
    const byOp = new Map(vectors.map((vector) => [vector.op, vector]));
    const invocations: {
      peerId: string;
      op: string;
      payload: Record<string, unknown>;
    }[] = [];

    const ports = {
      ...asBaselineOnly(new ToyWorldAdapter()),
      invoke: async (
        peerId: string,
        op: string,
        payload: Record<string, unknown>,
      ): Promise<SpokeResult<unknown>> => {
        invocations.push({ peerId, op, payload });
        const vector = byOp.get(op);
        if (vector === undefined) {
          return spokeReject(
            SpokeRejectCode.INTERNAL_ERROR,
            `no golden vector for ${op}`,
          );
        }
        return spokeOk(vector.response);
      },
    };

    const { responder, client, peerIdClient } = await dialWithResponder({
      ports,
      clientManifest: memoryManifest("test-client"),
      responderManifest: memoryManifest("test-responder"),
    });
    // note: the client-side `principal` is a local admission context — the host
    // derives it from the authenticated peer, so it never rides the wire.
    const principal = { principal_id: "prn_local_caller", extensions: {} };

    try {
      for (const vector of vectors) {
        const request = vector.request;
        let result: SpokeResult<unknown>;
        switch (vector.op) {
          case "memory.describe":
            result = await client.describe(
              principal,
              request as unknown as MemoryDescribeRequest,
            );
            break;
          case "memory.capture":
            result = await client.capture(
              principal,
              request as unknown as MemoryCaptureRequest,
            );
            break;
          case "memory.candidates.review":
            result = await client.reviewCandidate(
              principal,
              request as unknown as MemoryCandidatesReviewRequest,
            );
            break;
          case "memory.exports.get":
            result = await client.getExport(
              principal,
              request as unknown as MemoryExportsGetRequest,
            );
            break;
          case "memory.imports.stage":
            result = await client.stageImport(
              principal,
              request as unknown as MemoryImportsStageRequest,
            );
            break;
          case "memory.receipts.get":
            result = await client.getReceipt(
              principal,
              request as unknown as MemoryReceiptsGetRequest,
            );
            break;
          default:
            throw new Error(`no adapter arm for ${vector.op}`);
        }
        expect(result.ok, `${vector.op} must succeed`).toBe(true);
        if (result.ok) {
          expect(result.value, `${vector.op} response`).toEqual(vector.response);
        }
      }

      expect(invocations).toHaveLength(vectors.length);
      for (const [index, vector] of vectors.entries()) {
        expect(invocations[index].op).toBe(vector.op);
        expect(invocations[index].peerId).toBe(peerIdClient);
        expect(invocations[index].payload).toEqual(vector.request);
        expect(invocations[index].payload.requester_principal_id).toBeUndefined();
      }
    } finally {
      client.close();
      responder.close();
    }
  });

  it("answers capability_missing for an unnegotiated memory op without calling the service", async () => {
    const captureVector = memoryVectors().find(
      (vector) => vector.op === "memory.capture",
    );
    expect(captureVector).toBeDefined();
    const vector = captureVector as MemoryVector;
    let calls = 0;
    const ports = {
      ...asBaselineOnly(new ToyWorldAdapter()),
      invoke: async (): Promise<SpokeResult<unknown>> => {
        calls += 1;
        return spokeOk(vector.response);
      },
    };
    // why: neither manifest declares the memory family, so the gate denies before
    // the service is probed.
    const { responder, client } = await dialWithResponder({ ports });

    try {
      const result = await client.capture(
        { principal_id: "prn_local_caller", extensions: {} },
        vector.request as unknown as MemoryCaptureRequest,
      );
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
        expect(result.details?.wire_code).toBe("capability_missing");
        // note: per the protocol §14.2 rule, a memory-boundary deny is classified terminal.
        expect(result.details?.retry_class).toBe("terminal");
      }
      expect(calls).toBe(0);
    } finally {
      client.close();
      responder.close();
    }
  });

  it("answers an unknown memory op with op_unsupported plus the terminal retry class", async () => {
    // why: protocol §14.2 — the gate denies before the memory service is probed,
    // so the terminal classification must ride the dispatch-deny envelope itself.
    const { responder, clientEnd, seedClient, pubkeyResponder, peerIdResponder } =
      await startRawResponder({ manifest: memoryManifest("test-responder") });
    try {
      const { session_id: sessionId } = await rawHandshake(clientEnd, {
        seed: seedClient,
        manifest: memoryManifest("test-client"),
        pubkeyResponder,
        peerIdResponder,
      });
      const unknownMemory = await signInvokeRequest(seedClient, {
        session_id: sessionId,
        sequence: 0,
        request_id: "unknown-memory-op",
        op: "memory.not_an_op",
        payload: {},
      });
      await clientEnd.send(encodeEnvelope(unknownMemory));
      const memoryDeny = decodeEnvelope(await clientEnd.recv()) as {
        error: { code: string; details?: { retry_class?: string } };
      };
      expect(memoryDeny.error.code).toBe("op_unsupported");
      expect(memoryDeny.error.details?.retry_class).toBe("terminal");

      // note: control — the classification stays memory-scoped, so an unknown op
      // of another family keeps its bare `op_unsupported` envelope.
      const unknownOther = await signInvokeRequest(seedClient, {
        session_id: sessionId,
        sequence: 1,
        request_id: "unknown-non-memory-op",
        op: "photosynthesis.derive",
        payload: {},
      });
      await clientEnd.send(encodeEnvelope(unknownOther));
      const otherDeny = decodeEnvelope(await clientEnd.recv()) as {
        error: { code: string; details?: unknown };
      };
      expect(otherDeny.error.code).toBe("op_unsupported");
      expect(otherDeny.error.details).toBeUndefined();
    } finally {
      responder.close();
    }
  }, 15000);

  it("reports OUTCOME_UNCERTAIN when the memory budget expires mid-service and recovers on replay", async () => {
    const vector = memoryVectors().find((entry) => entry.op === "memory.capture");
    expect(vector).toBeDefined();
    const capture = vector as MemoryVector;
    const request = {
      ...capture.request,
      idempotency_key: "cap-uncertain",
    } as unknown as MemoryCaptureRequest;
    const committed = { ...capture.response, idempotency_key: "cap-uncertain" };
    let commits = 0;
    const ports = {
      ...asBaselineOnly(new ToyWorldAdapter()),
      invoke: async (
        _peerId: string,
        op: string,
        payload: Record<string, unknown>,
      ): Promise<SpokeResult<unknown>> => {
        expect(op).toBe("memory.capture");
        expect(payload.idempotency_key).toBe("cap-uncertain");
        if (commits === 0) {
          // why: the provider commits after the responder's budget expires, so the
          // transport cannot know whether the mutation landed.
          await delay(150);
          commits += 1;
        }
        return spokeOk(committed);
      },
    };
    const { responder, client } = await dialWithResponder({
      ports,
      responderTimeoutMs: 30,
      clientManifest: memoryManifest("test-client"),
      responderManifest: memoryManifest("test-responder"),
    });

    try {
      const timedOut = await client.capture(
        { principal_id: "prn_local_caller", extensions: {} },
        request,
      );
      expect(timedOut.ok).toBe(false);
      if (!timedOut.ok) {
        expect(timedOut.code).toBe(SpokeRejectCode.OUTCOME_UNCERTAIN);
        expect(timedOut.details?.retry_class).toBe("uncertain");
      }

      await delay(250);
      const recovered = await client.capture(
        { principal_id: "prn_local_caller", extensions: {} },
        request,
      );
      expect(recovered.ok).toBe(true);
      if (recovered.ok) {
        expect(recovered.value).toEqual(committed);
      }
    } finally {
      client.close();
      responder.close();
    }
  });

  it("reports OUTCOME_UNCERTAIN when the memory service throws after invocation", async () => {
    // why: protocol §14.3 parity with the Rust detached-invoke panic — the provider
    // died after admission, so a clean INTERNAL_ERROR would assert no-effect.
    const vector = memoryVectors().find((entry) => entry.op === "memory.capture");
    expect(vector).toBeDefined();
    const request = {
      ...(vector as MemoryVector).request,
      idempotency_key: "cap-panic",
    } as unknown as MemoryCaptureRequest;
    const ports = {
      ...asBaselineOnly(new ToyWorldAdapter()),
      invoke: async (): Promise<SpokeResult<unknown>> => {
        throw new Error("provider exploded after admission");
      },
    };
    const { responder, client } = await dialWithResponder({
      ports,
      clientManifest: memoryManifest("test-client"),
      responderManifest: memoryManifest("test-responder"),
    });

    try {
      const failed = await client.capture(
        { principal_id: "prn_local_caller", extensions: {} },
        request,
      );
      expect(failed.ok).toBe(false);
      if (!failed.ok) {
        expect(failed.code).toBe(SpokeRejectCode.OUTCOME_UNCERTAIN);
        expect(failed.details?.retry_class).toBe("uncertain");
        expect(failed.details?.kind).toBe("panic");
      }
    } finally {
      client.close();
      responder.close();
    }
  });
});
