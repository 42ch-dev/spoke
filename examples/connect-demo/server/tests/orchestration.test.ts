/**
 * DemoOrchestrator integration tests — the library `connectResponder`
 * (dogfooded responder) + `DemoOrchestrator` over `loopbackTransportPair()`
 * with the real `connectRemoteAdapter`:
 *
 *   - discovery from the authenticated manifest (both frozen tools listed);
 *   - mid-orchestration reverse invoke of roll_dice (served by the dialer's
 *     registered handler) feeding the roll result into the engine as a
 *     knowledge entry (a BaselinePorts step);
 *   - the negative path: a dialer that does not negotiate the tool gets
 *     CAPABILITY_PORT_MISSING (wire_code op_unsupported) — recorded, nothing
 *     fed, no silent success.
 *
 * The real-WebSocket e2e (client/tests/e2e.test.ts) exercises the same
 * story over the wire; this suite keeps the server package's own coverage of
 * its responder wiring (the hand-rolled responder tests were removed with
 * the switch to `connectResponder`).
 */

import { describe, expect, it } from "vitest";

import type { HostCapabilityManifest, SourceAnchor } from "@42ch/spoke-schemas";
import {
  connectRemoteAdapter,
  connectResponder,
  loopbackTransportPair,
  type RemoteAdapter,
} from "@42ch/spoke-connect/remote";

import {
  DEMO_EXTRACTION_CANARY,
  DEMO_SERVER_MANIFEST,
  MockAdapter,
} from "../src/adapter/mock-adapter.js";
import {
  DICE_ROLL_ENTRY_ID,
  DICE_ROLL_TRIGGER_ENTRY_ID,
  DemoOrchestrator,
  type DemoOrchestration,
} from "../src/host/orchestration.js";
import {
  DEMO_CLIENT_PEER_ID,
  DEMO_CLIENT_PUBKEY,
  DEMO_CLIENT_SEED,
  DEMO_SERVER_PEER_ID,
  DEMO_SERVER_PUBKEY,
  DEMO_SERVER_SEED,
} from "../src/identities.js";
import { DEMO_SCOPE_ID } from "../src/engine/seed-corpus.js";
import {
  LORE_LOOKUP_DESCRIPTOR,
  ROLL_DICE_DESCRIPTOR,
  TOY_WORLD_LORE_LOOKUP_ID,
  TOY_WORLD_NAMESPACE,
  TOY_WORLD_ROLL_DICE_ID,
} from "../src/tools/toy-world-tools.js";

/** The deterministic 2d6 roll the orchestration asserts (client handler parity). */
const EXPECTED_DICE_ROLL = { rolls: [1, 2], total: 3 };

/** Client manifest with the two frozen tools (same shape as the demo client). */
const TOOLS_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: [
    "spoke-baseline",
    TOY_WORLD_ROLL_DICE_ID,
    TOY_WORLD_LORE_LOOKUP_ID,
  ],
  namespaces: [DEMO_SCOPE_ID, TOY_WORLD_NAMESPACE],
  tools: [ROLL_DICE_DESCRIPTOR, LORE_LOOKUP_DESCRIPTOR],
  extensions: {},
};

/** Tools-less client manifest — the negative-path dialer. */
const MINIMAL_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: ["spoke-baseline"],
  namespaces: [DEMO_SCOPE_ID],
  extensions: {},
};

/**
 * Client manifest that negotiates the host's `ke-extraction` capability (the
 * negotiated set is the both-hello intersection, so the dialer must declare
 * the flag too).
 */
const EXTRACT_CLIENT_MANIFEST: HostCapabilityManifest = {
  ...TOOLS_CLIENT_MANIFEST,
  capabilities: [
    ...TOOLS_CLIENT_MANIFEST.capabilities,
    "ke-extraction",
  ] as HostCapabilityManifest["capabilities"],
};

/** The extraction request the responder round-trips through the wrapper. */
const EXTRACT_RUN_ID = "demo-run/orchestration-1";
const EXTRACT_SOURCES: [SourceAnchor, ...SourceAnchor[]] = [
  {
    schema_version: 1,
    source_id: "demo-harbor/manuscript/harbor-chapter-1",
    label: "Harbor chapter 1",
    extensions: {},
  },
];

/** The compass submission that triggers the orchestration. */
const COMPASS_ENTRY = {
  schema_version: 1,
  entry_id: DICE_ROLL_TRIGGER_ENTRY_ID,
  entry_type: "item",
  canonical_name: "Compass",
  status: "provisional",
  body: { summary: "A brass compass." },
  extensions: {},
};

async function dialOrchestrated(
  options: {
    manifest?: HostCapabilityManifest;
    registerRollDice?: boolean;
  } = {},
): Promise<{
  records: DemoOrchestration[];
  listedIds: string[];
  client: RemoteAdapter;
  close: () => void;
}> {
  const records: DemoOrchestration[] = [];
  const adapter = new MockAdapter();
  const orchestrator = new DemoOrchestrator(adapter, records);

  const pair = loopbackTransportPair();
  const responder = await connectResponder({
    transport: pair.server,
    identity: { seed: DEMO_SERVER_SEED },
    manifest: DEMO_SERVER_MANIFEST,
    allowlist: [DEMO_CLIENT_PEER_ID],
    peerKeys: { [DEMO_CLIENT_PEER_ID]: DEMO_CLIENT_PUBKEY },
    ports: orchestrator,
  });
  orchestrator.setResponder(responder);

  const client = await connectRemoteAdapter({
    transport: pair.client,
    localIdentity: { seed: DEMO_CLIENT_SEED },
    localManifest: options.manifest ?? TOOLS_CLIENT_MANIFEST,
    remotePubkey: DEMO_SERVER_PUBKEY,
    allowlist: [DEMO_SERVER_PEER_ID],
  });
  if (options.registerRollDice ?? true) {
    // The deterministic roll_dice handler (same algorithm as the demo
    // client's copy of the toy-world fixture handler).
    client.registerToolHandler(TOY_WORLD_ROLL_DICE_ID, async () => ({
      ok: true as const,
      value: { ...EXPECTED_DICE_ROLL },
    }));
  }

  const created = await client.putKnowledgeEntry(COMPASS_ENTRY, null);
  expect(created.ok).toBe(true);

  const listed = await client.listKnowledgeEntries({ scope_id: DEMO_SCOPE_ID });
  expect(listed.ok).toBe(true);

  return {
    records,
    listedIds: listed.ok
      ? listed.value.map((entry) => entry.entry_id)
      : [],
    client,
    close: () => {
      client.close();
      responder.close();
    },
  };
}

describe("DemoOrchestrator tool-assisted orchestration", () => {
  it("discovers the client tools, reverse-invokes roll_dice, and feeds the roll result into the engine", async () => {
    const run = await dialOrchestrated();
    try {
      expect(run.records).toHaveLength(1);
      const [record] = run.records;
      expect(record.discovered).toEqual([
        TOY_WORLD_ROLL_DICE_ID,
        TOY_WORLD_LORE_LOOKUP_ID,
      ]);
      expect(record.manifestValid).toBe(true);
      expect(record.tool_id).toBe(TOY_WORLD_ROLL_DICE_ID);
      expect(record.args).toEqual({ count: 2, sides: 6 });
      expect(record.result).toEqual({ ok: true, value: EXPECTED_DICE_ROLL });
      expect(record.fed_entry_id).toBe(DICE_ROLL_ENTRY_ID);

      // The fed entry is a BaselinePorts step the client sees: it appears in
      // the engine's list, carrying the exact roll result.
      expect(run.listedIds).toContain(DICE_ROLL_ENTRY_ID);
    } finally {
      run.close();
    }
  });

  it("records the deny when the client does not negotiate the tool (no silent success)", async () => {
    const run = await dialOrchestrated({
      manifest: MINIMAL_CLIENT_MANIFEST,
      registerRollDice: false,
    });
    try {
      expect(run.records).toHaveLength(1);
      const [record] = run.records;
      // No tools were discovered in the authenticated manifest; the
      // manifest itself is still valid (no tools to violate rules).
      expect(record.discovered).toEqual([]);
      expect(record.manifestValid).toBe(true);
      expect(record.tool_id).toBe(TOY_WORLD_ROLL_DICE_ID);
      expect(record.result.ok).toBe(false);
      if (!record.result.ok) {
        expect(record.result.code).toBe("CAPABILITY_PORT_MISSING");
        expect(record.result.details?.wire_code).toBe("op_unsupported");
      }
      expect(record.fed_entry_id).toBeUndefined();
      expect(run.listedIds).not.toContain(DICE_ROLL_ENTRY_ID);
    } finally {
      run.close();
    }
  });
});

describe("DemoOrchestrator extract service (ke-extraction)", () => {
  it("serves extract through the injected orchestrator the responder holds", async () => {
    const run = await dialOrchestrated({ manifest: EXTRACT_CLIENT_MANIFEST });
    try {
      const result = await run.client.extract({
        run_id: EXTRACT_RUN_ID,
        sources: EXTRACT_SOURCES,
      });
      expect(result.ok, result.ok ? undefined : result.message).toBe(true);
      if (!result.ok) {
        return;
      }
      // The library maps a wire error branch to a reject, so a success result
      // holds the candidates/run branch at runtime; the union still needs the
      // structural narrowing.
      expect("error" in result.value).toBe(false);
      if (!("candidates" in result.value)) {
        throw new Error("expected the extract success branch");
      }

      expect(result.value.run.run_id).toBe(EXTRACT_RUN_ID);
      expect(result.value.candidates).toHaveLength(EXTRACT_SOURCES.length);
      expect(result.value.candidates[0].status).toBe("provisional");
      expect(result.value.candidates[0].source_anchor).toEqual(
        EXTRACT_SOURCES[0],
      );
      // The host-local loaded value stays host-local: the response carries no
      // loader canary. T2 repeats this over the captured WebSocket frames.
      expect(JSON.stringify(result.value)).not.toContain(
        DEMO_EXTRACTION_CANARY,
      );
    } finally {
      run.close();
    }
  });

  it("denies extract when the dialer does not negotiate ke-extraction", async () => {
    // MINIMAL_CLIENT_MANIFEST omits the flag, so the both-hello intersection
    // lacks it: the responder's static core-table gate denies before the
    // service probe and nothing is served.
    const run = await dialOrchestrated({
      manifest: MINIMAL_CLIENT_MANIFEST,
      registerRollDice: false,
    });
    try {
      const result = await run.client.extract({
        run_id: EXTRACT_RUN_ID,
        sources: EXTRACT_SOURCES,
      });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe("CAPABILITY_PORT_MISSING");
        expect(result.details?.wire_code).toBe("op_unsupported");
      }
    } finally {
      run.close();
    }
  });
});
