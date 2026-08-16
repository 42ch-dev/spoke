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
 *   - the allowlist negative proof (stranger dial rejected server-side).
 */

import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";

import { connectRemoteAdapter } from "@42ch/spoke-connect/remote";
import type { HostCapabilityManifest } from "@42ch/spoke-schemas";
import {
  DICE_ROLL_ENTRY_ID,
  DEMO_SEED_ENTRIES,
  DEMO_SERVER_MANIFEST,
  DERIVED_WORLD_DIGEST_ENTRY_ID,
  serveConnectDemo,
  type ServeConnectDemoHandle,
} from "@42ch/spoke-demo-server";

import {
  DEMO_CLIENT_MANIFEST,
  runDemoClient,
} from "../src/main.js";
import {
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
 * Tools-less client manifest — the negative tool e2e: this client does not
 * negotiate any tool, so the host's mid-orchestration reverse invoke must be
 * denied by the protocol (capability gate → op_unsupported →
 * CAPABILITY_PORT_MISSING).
 */
const MINIMAL_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: ["spoke-baseline"],
  namespaces: [DEMO_SCOPE_ID],
  extensions: {},
};

let server: ServeConnectDemoHandle;
const transports: WsTransport[] = [];

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

    // listKnowledgeEntries contains the seed corpus + the submitted entry +
    // the engine-derived world-digest artifact + the orchestration's
    // dice-roll artifact (the roll result fed a BaselinePorts step).
    const expectedIds = [
      ...DEMO_SEED_ENTRIES.map((entry) => entry.entry_id),
      DERIVED_WORLD_DIGEST_ENTRY_ID,
      DICE_ROLL_ENTRY_ID,
      run.created.entry_id,
    ].sort();
    expect(run.listed.map((entry) => entry.entry_id).sort()).toEqual(
      expectedIds,
    );

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
