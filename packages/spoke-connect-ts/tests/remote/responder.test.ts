/**
 * `connectResponder` (Task 2 of tools-connect-reverse-invoke) — loopback
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

import type {
  ConnectHello,
  ConnectInvokeRequest,
  HostCapabilityManifest,
  KnowledgeEntry,
  ToolDescriptor,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  spokeOk,
  validateManifestTools,
  type BaselinePorts,
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
import { ToyWorldAdapter } from "@42ch/spoke-fixture-toy-world";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import {
  authenticateInvokeRequest,
  authenticateInvokeResponse,
} from "../../src/core/envelope-auth.js";
import { verifyHelloEd25519 } from "../../src/core/hello.js";
import { generateNonce } from "../../src/core/nonce.js";
import { signHelloEd25519 } from "../../src/core/hello.js";
import { schemaConformantManifest } from "../../src/golden.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  connectResponder,
  type ConnectResponder,
  type ConnectResponderState,
} from "@42ch/spoke-connect/remote";

/** Fixture seed: base+i, all values within byte range for base ≤ 0xe0. */
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

/** Tool-carrying manifest: namespaces own the tool namespaces; every tool capability ∈ capabilities[]. */
function toolManifest(hostId: string): HostCapabilityManifest {
  return {
    ...schemaConformantManifest(),
    host_id: hostId,
    namespaces: ["toy_world", "math", "echo"],
    capabilities: [
      "spoke-baseline",
      "tools.math.add",
      "tools.echo.echo",
      "tools.echo.boom",
    ],
    tools: [ADD_DESCRIPTOR, ECHO_DESCRIPTOR, BOOM_DESCRIPTOR],
  };
}

/** The add handler used by most fixtures. */
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
  ports?: BaselinePorts;
  /** Bounded wait for the RESPONDER's reverse-invoke waiters, ms. */
  responderTimeoutMs?: number;
  /** Bounded wait for the CLIENT's dial + invoke waiters, ms. */
  clientTimeoutMs?: number;
  clientTransport?: (clientEnd: Transport) => Transport;
  responderTransport?: (serverEnd: Transport) => Transport;
}

/**
 * Loopback pair: `connectResponder` (server end) + real T1
 * `connectRemoteAdapter` (client end). The responder's handshake runs in
 * the background; the client's dial is the synchronization point.
 */
async function dialWithResponder(
  options: DialResponderOptions = {},
): Promise<{
  responder: ConnectResponder;
  client: RemoteAdapter;
  pair: ReturnType<typeof loopbackTransportPair>;
  peerIdResponder: string;
  peerIdClient: string;
  /** The dialer's Ed25519 seed (wire-level re-sign fixtures). */
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
  // Fixture hygiene: both manifests satisfy the manifest-tools rules.
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
    ports: options.ports,
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

/** Decode wire bytes to a JSON document (wire-level injector fixtures). */
function decodeWire(bytes: EnvelopeBytes): Record<string, unknown> {
  return decodeJsonMessage(bytes) as Record<string, unknown>;
}

/** Bounded wall-clock wait (real-timer integration fixtures only). */
function delay(ms: number): Promise<void> {
  // Executor form: the project's TS lib target predates
  // `Promise.withResolvers` (ES2024), so executor form is required here.
  return new Promise<void>((resolve) => {
    setTimeout(resolve, ms);
  });
}

/** Bounded poll for an async state transition (loopback close propagation). */
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

// ── raw-wire helpers (unknown-port / gate fixtures) ────────────────────────

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

function decodeEnvelope(bytes: EnvelopeBytes): unknown {
  return decodeJsonMessage(bytes);
}

/** Start a responder WITHOUT dialing (raw-wire tests drive the wire). */
async function startRawResponder(options: {
  ports?: BaselinePorts;
  manifest?: HostCapabilityManifest;
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
    ports: options.ports ?? ToyWorldAdapter.withCommittedFixtures(),
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

/** Raw initiator handshake (the real library client is exercised elsewhere). */
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

/** Sign a raw wire `ConnectInvokeRequest` over the locked 5-field set. */
async function signInvokeRequest(
  seed: Uint8Array,
  request: {
    session_id: string;
    sequence: number;
    request_id: string;
    op: string;
    payload: Record<string, unknown>;
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
        // Session peer binding: the responder's remote peer is the dialer.
        expect(responder.remotePeerId).toBe(peerIdClient);
        expect(client.remotePeerId).toBe(peerIdResponder);
        // Discovery after auth: the authenticated hello `host` is the
        // source — the responder sees the dialer's tools[] only once the
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
        allowlist: [peerIdStranger], // allow the STRANGER (client-side trusts the server)
        peerKeys: {
          [peerIdStranger]: getPublicKeyEd25519(seedStranger),
        },
        ports: new ToyWorldAdapter(),
      });
      try {
        // The server-side allowlist rejects the hello and closes the
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
        // port.knowledge.get — seeded fixture round-trip.
        const mira = await client.getKnowledgeEntry(MIRA_ENTRY_ID);
        expect(mira.ok).toBe(true);
        if (mira.ok) {
          expect(mira.value.entry_id).toBe(MIRA_ENTRY_ID);
        }

        // port.knowledge.put — create (expected_base_revision null), then
        // a compare-and-swap update over the wire (the toy-world store
        // treats an absent revision as 0, so base 0 accepts the update).
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

        // Negative OCC over the wire: re-creating an existing entry rejects
        // REVISION_CONFLICT through the responder's error branch.
        const conflicted = await client.putKnowledgeEntry(compass, null);
        expect(conflicted.ok).toBe(false);
        if (!conflicted.ok) {
          expect(conflicted.code).toBe(SpokeRejectCode.REVISION_CONFLICT);
        }

        // port.scope.list_knowledge_entries — includes the created entry.
        const listed = await client.listKnowledgeEntries({
          scope_id: "toy-scope-001",
        });
        expect(listed.ok).toBe(true);
        if (listed.ok) {
          expect(
            listed.value.some((entry) => entry.entry_id === compass.entry_id),
          ).toBe(true);
        }

        // port.host.list_peer_manifests — the adapter's product-seeded peers.
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
        // A wire-valid, properly signed invoke for an op outside the D4
        // catalogue: the dispatch gate denies it (no core row, no product
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
      // No `ports` injected: the capability gate passes (spoke-baseline is
      // negotiated) but there is no BaselinePorts to serve — the responder
      // answers the dispatch-deny branch, mapped by the D7 invoker row.
      const { responder, client } = await dialWithResponder();
      try {
        const result = await client.getKnowledgeEntry(MIRA_ENTRY_ID);
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("no BaselinePorts configured"),
          details: { wire_code: "op_unsupported" },
        });
        // The session stays usable.
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
        // The dialer-side registered handler is what ran, with the
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
        // The tool IS negotiated but the dialer serves no handler for it —
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
      // The client manifest carries no tools: the negotiated set lacks the
      // tool capability, so the dialer's dispatch gate denies the reverse
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
      // T2-M1 (folded): the RESPONDER's reverse invokeTool { result } gate.
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
          // The malformed payload fails only this waiter — the session
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
      // The dialer's handler never resolves: the request DID hit the wire
      // (outbound sequence transmitted), so the waiter times out but the
      // session stays usable on both ends — no poison-close.
      const { responder, client } = await dialWithResponder({
        responderTimeoutMs: 100,
      });
      try {
        // Never-settling handler: the request DID hit the wire, so the
        // waiter times out but the session stays usable (no poison-close).
        client.registerToolHandler(
          "tools.echo.boom",
          // Executor form: `Promise.withResolvers` needs the ES2024 lib
          // target; a never-settling promise has no resolvers to call.
          () => new Promise<never>(() => {}),
        );
        const timedOut = await responder.invokeTool("tools.echo.boom", {});
        expect(timedOut).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        // The session stays usable: a follow-up reverse invoke for a
        // resolving handler succeeds.
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
      // Wire-level injector on the RESPONDER end: the FIRST outbound
      // reverse invoke's send is delayed on the wire; the second reverse
      // invoke's send is serialized behind it (send tail) and times out
      // while waiting — before its send ever starts. When the first send
      // finally completes, the queued send is skipped: its waiter already
      // settled, so transmitting it late would be a duplicate dispatch on
      // the dialer. The skip must instead close the session (the allocated
      // outbound sequence never hit the wire — the dialer's inbound gate
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
                // Real-clock delay is deliberate (integration exception):
                // the race under test IS between the responder's invoke
                // timeout timer and a slow transport send, with both peers
                // on real timers — fake timers cannot drive the loopback
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
          responder.invokeTool("tools.math.add", { a: 1, b: 2 }), // sequence 0 — send delayed
          responder.invokeTool("tools.math.add", { a: 3, b: 4 }), // sequence 1 — times out queued behind it
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
        // The delayed first send completes, the queued second send is
        // skipped, and the skip closes the session.
        await untilState(responder, "Closed", "responder");
        // Only the first reverse invoke ever reached the wire; the
        // timed-out-while-queued second invoke was never transmitted.
        expect(sentSequences).toEqual([0]);
        // The session is closed, not poisoned: a follow-up reverse invoke
        // fails with session_closed instead of hanging or being
        // mis-rejected by the dialer's stuck inbound gate.
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
        // Loop damage check: the responder's serve loop survived — a
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
      // Disjoint capability sets: the negotiated intersection is empty, so
      // the responder's signed session snapshot must fall back to
      // `["spoke-baseline"]` (wire minItems 1). The dialer computes its own
      // intersection for gating, so the fallback has no authorization
      // impact — the dial must simply establish.
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
        // A frame that fails JSON decode is a protocol violation: the
        // responder's serve loop must actually close the transport (the
        // carried-over demo behavior) — a bare return would leave the
        // client's established session hanging on its next invoke.
        await pair.client.send(new TextEncoder().encode("not json {{{"));
        await untilState(responder, "Closed", "responder");
        // The dialer observes transport loss and closes too.
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
        // A wire-valid, properly signed invoke at a non-expected sequence:
        // the peek fails — invalid_sequence, counter unchanged.
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

        // The inbound counter is still at 0: a valid invoke at sequence 0
        // dispatches and succeeds.
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
        // Wire-level tamper: mutate the payload AFTER signing — the
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

        // Auth-before-advance: the inbound counter is UNCHANGED, so the
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
