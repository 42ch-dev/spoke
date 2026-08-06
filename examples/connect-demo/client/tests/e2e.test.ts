/**
 * The connect demo end-to-end gate (plan T3): boots the REAL demo server
 * (`serveConnectDemo`) on an ephemeral port, dials it over a REAL WebSocket
 * with the REAL library client (`connectRemoteAdapter`), and asserts the
 * full third-party story plus the negative allowlist proof.
 *
 * This is the failing test the task is built against — it only passes once
 * the WebSocket transports (both ends), the CLIs' programmatic surface, and
 * the client flow all exist and interoperate.
 */

import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";

import { connectRemoteAdapter } from "@42ch/spoke-connect/remote";
import {
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
  DEMO_STRANGER_SEED,
} from "../src/identities.js";
import { WsTransport } from "../src/transport/ws-transport.js";

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

    // put → get round-trip with OCC: create (revision 1), compare-and-swap
    // update (revision 2), then fetch the updated entry back.
    expect(run.created.revision).toBe(1);
    expect(run.updated.revision).toBe(2);
    expect(run.updated.status).toBe("confirmed");
    expect(run.fetched).toEqual(run.updated);

    // listKnowledgeEntries contains the seed corpus + the submitted entry +
    // the engine-derived world-digest artifact.
    const expectedIds = [
      ...DEMO_SEED_ENTRIES.map((entry) => entry.entry_id),
      DERIVED_WORLD_DIGEST_ENTRY_ID,
      run.created.entry_id,
    ].sort();
    expect(run.listed.map((entry) => entry.entry_id).sort()).toEqual(
      expectedIds,
    );

    // putFindings round-trips the submitted finding.
    expect(run.findings).toHaveLength(1);
    expect(run.findings[0].target_entry_id).toBe(run.created.entry_id);

    // The demo host knows no peers — empty list is valid (spec D5).
    expect(run.peerManifests).toEqual([]);

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
