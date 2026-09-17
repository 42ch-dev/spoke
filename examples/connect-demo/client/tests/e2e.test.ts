/**
 * The connect demo end-to-end gate (plan T3 + Task 2 reverse-tool e2e):
 * boots the REAL demo server (`serveConnectDemo`) on an ephemeral port,
 * dials it over a REAL WebSocket with the REAL library client
 * (`connectRemoteAdapter`), and asserts the full third-party story:
 *
 *   - the client exposes two deterministic toy-world tools (roll_dice +
 *     lore_lookup) on its dial;
 *   - the host lists those tools from the authenticated manifest and
 *     reverse-invokes roll_dice mid-orchestration, feeding the roll result
 *     into a BaselinePorts step (a knowledge entry the client sees on its
 *     next list);
 *   - the negative path: a client that does not negotiate the tool gets a
 *     capability deny (CAPABILITY_PORT_MISSING / op_unsupported) — the host
 *     does not succeed silently;
 *   - the optional families over the same real WebSocket: l2-computable
 *     (project → compute settle → derived state) and l5-fork
 *     (listForkTimelineEvents over the seeded storm fork);
 *   - the optional-capability deny negative: a server variant whose
 *     manifest omits l2-computable denies compute client-side
 *     (CAPABILITY_PORT_MISSING / details.wire_code op_unsupported);
 *   - ke-extraction over the same real WebSocket: the reference-only request
 *     goes out, the host's own loader/extractor answer provisional
 *     run-correlated candidates, and the host-local loader canary never
 *     appears in the captured request/response frames;
 *   - the ke-extraction capability-gate negative (server manifest without the
 *     flag) and the ke-ownership negative for a viewpoint-bearing scope
 *     query, both answered by the real responder as op_unsupported;
 *   - the ke-ownership allow: the viewpoint-bearing listing returns the
 *     shared and the holder's own private fixture (governance intact) and
 *     withholds the foreign holder's;
 *   - the allowlist negative proof (stranger dial rejected server-side).
 */

import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";

import {
  connectRemoteAdapter,
  type EnvelopeBytes,
  type RemoteAdapter,
  type Transport,
} from "@42ch/spoke-connect/remote";
import type { HostCapabilityManifest } from "@42ch/spoke-schemas";
import {
  DICE_ROLL_ENTRY_ID,
  DEMO_EXTRACTION_CANARY,
  DEMO_EXTRACTION_METHOD,
  DEMO_FOREIGN_PRIVATE_ENTRY_ID,
  DEMO_HOLDER_ENTRY_ID,
  DEMO_OWN_PRIVATE_ENTRY_ID,
  DEMO_SEED_FORK_ID,
  DEMO_SEED_TIMELINE_EVENTS,
  DEMO_SERVER_MANIFEST,
  DEMO_SHARED_ENTRY_ID,
  DERIVED_WORLD_DIGEST_ENTRY_ID,
  demoExtractCandidateEntryId,
  serveConnectDemo,
  type ServeConnectDemoHandle,
} from "@42ch/spoke-demo-server";

import {
  DEMO_CLIENT_MANIFEST,
  DEMO_EXTRACTION_REQUEST,
  DEMO_EXTRACTION_RUN_ID,
  DEMO_STORM_FORK_ID,
  DEMO_VIEWPOINT_HOLDER_ID,
  runDemoClient,
} from "../src/main.js";
import {
  DEMO_CLIENT_SEED,
  DEMO_SERVER_PEER_ID,
  DEMO_SERVER_PUBKEY,
  DEMO_SCOPE_ID,
  DEMO_STRANGER_SEED,
} from "../src/identities.js";
import {
  TOY_WORLD_LORE_LOOKUP_ID,
  TOY_WORLD_ROLL_DICE_ID,
} from "../src/tools/toy-world-tools.js";
import { WsTransport } from "../src/transport/ws-transport.js";

/** The deterministic roll the host's orchestration gets for 2d6 (fixture parity). */
const EXPECTED_DICE_ROLL = { rolls: [1, 2], total: 3 };

/**
 * Tools-less client manifest — the negative tool e2e: this client negotiates
 * no tool, so the host's mid-orchestration reverse invoke must be denied by
 * the protocol (capability gate → op_unsupported →
 * CAPABILITY_PORT_MISSING). It still declares the two KE capability flags the
 * demo flow itself performs (the extract call and the viewpoint-bearing scope
 * query) so the flow reaches the reverse invoke instead of failing on its own
 * steps: capability flags are not tools, and this manifest still lists none.
 */
const MINIMAL_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: ["spoke-baseline", "ke-extraction", "ke-ownership"],
  namespaces: [DEMO_SCOPE_ID],
  extensions: {},
};

/**
 * Wire recorder: wraps the REAL demo `WsTransport` and retains every connect
 * envelope it carries in both directions. The adapter owns the receive loop,
 * so the wrapper sits underneath it and the canary scan below reads the bytes
 * that actually crossed the socket — never a re-serialized object.
 */
class RecordingTransport implements Transport {
  readonly frames: EnvelopeBytes[] = [];
  readonly #inner: Transport;

  constructor(inner: Transport) {
    this.#inner = inner;
  }

  async send(envelope: EnvelopeBytes): Promise<void> {
    this.frames.push(envelope);
    await this.#inner.send(envelope);
  }

  async recv(): Promise<EnvelopeBytes> {
    const envelope = await this.#inner.recv();
    this.frames.push(envelope);
    return envelope;
  }

  close(): void {
    this.#inner.close?.();
  }
}

let server: ServeConnectDemoHandle;
const transports: WsTransport[] = [];

/**
 * Boot a host variant whose manifest omits one capability and dial it with the
 * demo client manifest. The ports faces are unchanged, so any refusal is the
 * real responder's dispatch gate over the negotiated set — never a missing
 * implementation and never a client-side pre-gate.
 */
async function dialHostOmitting(capability: string): Promise<{
  denyServer: ServeConnectDemoHandle;
  adapter: RemoteAdapter;
}> {
  const denyServer = await serveConnectDemo({
    port: 0,
    manifest: {
      ...DEMO_SERVER_MANIFEST,
      // The schema types capabilities as a non-empty tuple; the variant keeps
      // the same family shape minus the omitted capability.
      capabilities: DEMO_SERVER_MANIFEST.capabilities.filter(
        (candidate) => candidate !== capability,
      ) as HostCapabilityManifest["capabilities"],
    },
  });
  try {
    const transport = new WsTransport(denyServer.url);
    transports.push(transport);
    const adapter = await connectRemoteAdapter({
      transport,
      localIdentity: { seed: DEMO_CLIENT_SEED },
      localManifest: DEMO_CLIENT_MANIFEST,
      remotePubkey: DEMO_SERVER_PUBKEY,
      allowlist: [DEMO_SERVER_PEER_ID],
    });
    return { denyServer, adapter };
  } catch (error) {
    denyServer.close();
    throw error;
  }
}

beforeAll(async () => {
  server = await serveConnectDemo({ port: 0 });
});

afterEach(() => {
  // Process hygiene: every transport created in this file is closed, even
  // when an assertion failed before the test's own cleanup ran.
  for (const transport of transports.splice(0)) {
    transport.close();
  }
});

afterAll(() => {
  server.close();
});

describe("connect demo over a real WebSocket", () => {
  it("completes the third-party RemoteAdapter flow end to end", async () => {
    const run = await runDemoClient({ url: server.url });
    transports.push(run.transport);

    // Remote manifest = the server's own manifest, cached at establish
    // (spec D5 — getHostCapabilityManifest is the session cache).
    expect(run.remotePeerId).toBe(server.peerId);
    expect(run.serverManifest).toEqual(DEMO_SERVER_MANIFEST);

    // The client exposed both frozen toy-world tools on its dial.
    expect(run.registeredToolIds).toEqual([
      TOY_WORLD_ROLL_DICE_ID,
      TOY_WORLD_LORE_LOOKUP_ID,
    ]);

    // put → get round-trip with OCC: create (revision 1), compare-and-swap
    // update (revision 2), then fetch the updated entry back.
    expect(run.created.revision).toBe(1);
    expect(run.updated.revision).toBe(2);
    expect(run.updated.status).toBe("confirmed");
    expect(run.fetched).toEqual(run.updated);

    // Step 3 — the ownership witness (ke-ownership / OQ-DEMO-1): the scope
    // query declares the demo viewpoint, so the host's disclosure predicate
    // decides visibility. The shared fixture and the holder's own private
    // fixture come back with governance intact; the foreign holder's private
    // fixture is withheld. The client keeps its own copy of the holder id —
    // this catches drift against the server's seed corpus.
    expect(DEMO_VIEWPOINT_HOLDER_ID).toBe(DEMO_HOLDER_ENTRY_ID);
    const listedIds = run.listed.map((entry) => entry.entry_id);
    expect(listedIds).toContain(DEMO_SHARED_ENTRY_ID);
    expect(listedIds).toContain(DEMO_OWN_PRIVATE_ENTRY_ID);
    expect(listedIds).not.toContain(DEMO_FOREIGN_PRIVATE_ENTRY_ID);
    // The rest of the listing is unchanged: the submitted entry and the
    // engine-derived artifacts are still visible to this viewpoint.
    expect(listedIds).toContain(run.created.entry_id);
    expect(listedIds).toContain(DERIVED_WORLD_DIGEST_ENTRY_ID);

    const ownPrivate = run.listed.find(
      (entry) => entry.entry_id === DEMO_OWN_PRIVATE_ENTRY_ID,
    );
    expect(ownPrivate?.owner).toBe(DEMO_HOLDER_ENTRY_ID);
    expect(ownPrivate?.disclosure).toBe("owner-private");
    // The unknown governance namespace round-trips verbatim — the query
    // neither strips nor reinterprets it.
    expect(ownPrivate?.extensions).toEqual({
      "demo-harbor": { retention_probe: "unknown-governance-field" },
    });

    // The dice-roll artifact carries the exact deterministic roll result —
    // proof the reverse-invoked tool result fed the engine.
    const diceRoll = run.listed.find(
      (entry) => entry.entry_id === DICE_ROLL_ENTRY_ID,
    );
    expect(diceRoll).toBeDefined();
    expect(diceRoll?.body.computable).toEqual(EXPECTED_DICE_ROLL);

    // putFindings round-trips the submitted finding.
    expect(run.findings).toHaveLength(1);
    expect(run.findings[0].target_entry_id).toBe(run.created.entry_id);

    // The demo host knows no peers — empty list is valid (spec D5).
    expect(run.peerManifests).toEqual([]);

    // ke-extraction (F1): the run through the demo client sent the
    // reference-only request and the host answered with run-correlated
    // provisional candidates — one per request anchor, ids derived from
    // `run_id` plus the source index, each carrying the request anchor as a
    // pointer rather than source content.
    expect(run.extractionRequest).toEqual(DEMO_EXTRACTION_REQUEST);
    expect(run.extraction.run.run_id).toBe(DEMO_EXTRACTION_RUN_ID);
    expect(run.extraction.run.method).toBe(DEMO_EXTRACTION_METHOD);
    expect(
      run.extraction.candidates.map((candidate) => candidate.entry_id),
    ).toEqual(
      DEMO_EXTRACTION_REQUEST.sources.map((_anchor, index) =>
        demoExtractCandidateEntryId(DEMO_EXTRACTION_RUN_ID, index),
      ),
    );
    expect(
      run.extraction.candidates.every(
        (candidate) => candidate.status === "provisional",
      ),
    ).toBe(true);
    expect(
      run.extraction.candidates.map((candidate) => candidate.source_anchor),
    ).toEqual(DEMO_EXTRACTION_REQUEST.sources);

    // The optional l2-computable round-trip: project materializes the
    // session's computable view from static state, compute applies the
    // delta and settles it back into static state (the derived state).
    // The default manifest declares the families, so the flow always runs.
    // The client's fork constant matches the server's seed corpus (the
    // client keeps its own copy — it must not import the server package).
    expect(DEMO_STORM_FORK_ID).toBe(DEMO_SEED_FORK_ID);
    expect(run.projected).toBeDefined();
    expect(run.computed).toBeDefined();
    expect(run.forkEvents).toBeDefined();
    expect(run.projected?.computable).toEqual({ ships_at_dock: 3 });
    expect(run.computed?.computable).toEqual({
      ships_at_dock: 3,
      tide: "rising",
    });
    expect(run.computed?.state).toEqual({ ships_at_dock: 3, tide: "rising" });

    // The optional l5-fork round-trip: the seeded storm-fork timeline
    // events come back verbatim over the real WebSocket.
    expect(run.forkEvents).toEqual(DEMO_SEED_TIMELINE_EVENTS);

    // An unknown fork id still round-trips — an empty timeline.
    const unknownFork = await run.adapter.listForkTimelineEvents({
      scope_id: DEMO_SCOPE_ID,
      fork_id: "demo-harbor/fork/unknown",
    });
    expect(unknownFork.ok).toBe(true);
    if (unknownFork.ok) {
      expect(unknownFork.value).toEqual([]);
    }

    // The seeded storm-fork timeline appears in the baseline listing too:
    // fork events live in the shared event store, and the fork-scoped query
    // is only their refinement — pin the full baseline set over the wire.
    const baselineTimeline = await run.adapter.listTimelineEvents({
      scope_id: DEMO_SCOPE_ID,
    });
    expect(baselineTimeline.ok).toBe(true);
    if (baselineTimeline.ok) {
      expect(baselineTimeline.value).toEqual(DEMO_SEED_TIMELINE_EVENTS);
    }

    run.close();
  });

  it("discovers the client tools from the authenticated manifest and reverse-invokes mid-orchestration", async () => {
    const baseline = server.orchestrations.length;
    const run = await runDemoClient({ url: server.url });
    transports.push(run.transport);

    // The host's orchestration record: discovery from the authenticated
    // manifest (both frozen tools, manifest order), then the reverse invoke,
    // then the fed entry.
    const records = server.orchestrations.slice(baseline);
    expect(records).toHaveLength(1);
    const [record] = records;
    expect(record.discovered).toEqual([
      TOY_WORLD_ROLL_DICE_ID,
      TOY_WORLD_LORE_LOOKUP_ID,
    ]);
    expect(record.tool_id).toBe(TOY_WORLD_ROLL_DICE_ID);
    expect(record.args).toEqual({ count: 2, sides: 6 });
    expect(record.result).toEqual({ ok: true, value: EXPECTED_DICE_ROLL });
    expect(record.fed_entry_id).toBe(DICE_ROLL_ENTRY_ID);

    run.close();
  });

  it("denies a reverse invoke for a tool the client does not list (capability deny)", async () => {
    const baseline = server.orchestrations.length;
    // This client negotiates no tools and registers no handlers — the host's
    // orchestration still attempts the roll and must surface the protocol
    // deny instead of succeeding silently.
    const run = await runDemoClient({
      url: server.url,
      manifest: MINIMAL_CLIENT_MANIFEST,
      registerTools: false,
    });
    transports.push(run.transport);

    expect(run.registeredToolIds).toEqual([]);
    expect(run.serverManifest).toEqual(DEMO_SERVER_MANIFEST);

    const records = server.orchestrations.slice(baseline);
    expect(records).toHaveLength(1);
    const [record] = records;
    // No tools were discovered in the authenticated manifest.
    expect(record.discovered).toEqual([]);
    expect(record.tool_id).toBe(TOY_WORLD_ROLL_DICE_ID);
    // The protocol denied the unlisted tool: op_unsupported → the
    // CAPABILITY_PORT_MISSING mapping. Nothing was fed into the engine.
    expect(record.result.ok).toBe(false);
    if (!record.result.ok) {
      expect(record.result.code).toBe("CAPABILITY_PORT_MISSING");
      expect(record.result.details?.wire_code).toBe("op_unsupported");
    }
    expect(record.fed_entry_id).toBeUndefined();

    // Client-visible proof: no dice-roll artifact exists in the engine.
    expect(
      run.listed.some((entry) => entry.entry_id === DICE_ROLL_ENTRY_ID),
    ).toBe(false);

    run.close();
  });

  it("carries the extract exchange over the real WebSocket without the host-local loader canary", async () => {
    // The recording wrapper sits around the REAL transport the adapter dials,
    // so every frame retained below is an actual socket payload.
    const raw = new WsTransport(server.url);
    transports.push(raw);
    const transport = new RecordingTransport(raw);
    const adapter = await connectRemoteAdapter({
      transport,
      localIdentity: { seed: DEMO_CLIENT_SEED },
      localManifest: DEMO_CLIENT_MANIFEST,
      remotePubkey: DEMO_SERVER_PUBKEY,
      allowlist: [DEMO_SERVER_PEER_ID],
    });

    try {
      const result = await adapter.extract(DEMO_EXTRACTION_REQUEST);
      expect(result.ok).toBe(true);
      if (!result.ok || "error" in result.value) {
        throw new Error("demo client: extract did not answer candidates");
      }
      expect(result.value.run.run_id).toBe(DEMO_EXTRACTION_RUN_ID);

      const wire = transport.frames
        .map((frame) => new TextDecoder().decode(frame))
        .join("\n");
      // Teeth: the frames scanned here really carry this exchange — the
      // outbound request's run id and the inbound candidate ids.
      expect(wire).toContain(DEMO_EXTRACTION_RUN_ID);
      expect(wire).toContain(
        demoExtractCandidateEntryId(DEMO_EXTRACTION_RUN_ID, 0),
      );
      // The loader value exists only inside the host's ExtractionPort; it is
      // absent from both directions of the wire.
      expect(wire).not.toContain(DEMO_EXTRACTION_CANARY);
    } finally {
      adapter.close();
    }
  });

  it("denies the extract core op when the server manifest omits ke-extraction", async () => {
    // The provider still serves extract: the refusal is the responder's
    // dispatch gate over the negotiated set (both hellos), and the wire
    // answer maps to the existing CAPABILITY_PORT_MISSING row.
    const { denyServer, adapter } = await dialHostOmitting("ke-extraction");
    try {
      const result = await adapter.extract(DEMO_EXTRACTION_REQUEST);
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe("CAPABILITY_PORT_MISSING");
        expect(result.details?.wire_code).toBe("op_unsupported");
      }
    } finally {
      adapter.close();
      denyServer.close();
    }
  });

  it("denies the viewpoint-bearing scope query when the server manifest omits ke-ownership", async () => {
    // A viewpoint-bearing Scope needs ke-ownership next to its row capability
    // (spoke-baseline, still negotiated here). The refusal must be the
    // responder's op_unsupported — never an empty success and never an
    // unfiltered list.
    const { denyServer, adapter } = await dialHostOmitting("ke-ownership");
    try {
      const result = await adapter.listKnowledgeEntries({
        scope_id: DEMO_SCOPE_ID,
        viewpoint: DEMO_VIEWPOINT_HOLDER_ID,
      });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe("CAPABILITY_PORT_MISSING");
        expect(result.details?.wire_code).toBe("op_unsupported");
      }
    } finally {
      adapter.close();
      denyServer.close();
    }
  });

  it("denies the optional compute op when the server manifest omits l2-computable", async () => {
    // The negotiated capability set lacks the family, so the responder's
    // dispatch gate denies port.computable.compute with wire op_unsupported
    // and the client maps it to CAPABILITY_PORT_MISSING (the existing D7
    // row). The deny must not succeed silently — the assertion is the
    // client-side mapped reject itself.
    const { denyServer, adapter } = await dialHostOmitting("l2-computable");
    try {
      const result = await adapter.compute({
        session_id: "demo-session/deny-negative",
        entry_id: "demo-harbor/location/harbor",
        computable: { tide: "rising" },
        settle: true,
      });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe("CAPABILITY_PORT_MISSING");
        expect(result.details?.wire_code).toBe("op_unsupported");
      }
    } finally {
      adapter.close();
      denyServer.close();
    }
  });

  it("rejects a dial from a non-allowlisted stranger identity", async () => {
    const transport = new WsTransport(server.url);
    transports.push(transport);

    // The stranger's OWN allowlist trusts the server, so the dial is
    // attempted; the SERVER-side allowlist rejects the hello and closes the
    // socket, failing the dial fast — no session is established. The
    // rejection is the handshake's connection loss (the server hung up
    // mid-dial), not a bare any-error assertion.
    await expect(
      connectRemoteAdapter({
        transport,
        localIdentity: { seed: DEMO_STRANGER_SEED },
        localManifest: DEMO_CLIENT_MANIFEST,
        remotePubkey: DEMO_SERVER_PUBKEY,
        allowlist: [DEMO_SERVER_PEER_ID],
      }),
    ).rejects.toThrow(/ws connection closed/);

    transport.close();
  });
});
