/**
 * The third-party client story (plan T3): `runDemoClient` dials a connect
 * host over a real WebSocket with the REAL library client
 * (`connectRemoteAdapter` from `@42ch/spoke-connect/remote`), executes the
 * demo flow through the drop-in async `BaselinePorts` surface, and returns
 * the asserted results. The CLI (`node dist/main.js --url ws://…`) prints
 * each story step.
 *
 * The client NEVER touches session-core verification helpers — it only
 * implements `Transport` and calls `connectRemoteAdapter` + `BaselinePorts`
 * (spec D10 encapsulation).
 */

import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
} from "@42ch/spoke-schemas";
import {
  connectRemoteAdapter,
  type RemoteAdapter,
} from "@42ch/spoke-connect/remote";

import {
  DEMO_CLIENT_PEER_ID,
  DEMO_CLIENT_SEED,
  DEMO_SERVER_PEER_ID,
  DEMO_SERVER_PUBKEY,
  DEMO_SCOPE_ID,
} from "./identities.js";
import { WsTransport } from "./transport/ws-transport.js";

/** The third-party app's own manifest (distinct from the server's). */
export const DEMO_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: ["spoke-baseline"],
  namespaces: [DEMO_SCOPE_ID],
  extensions: {},
};

/** The knowledge entry the third-party app submits (deterministic demo content). */
const SUBMITTED_ENTRY: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "demo-harbor/item/compass",
  entry_type: "item",
  canonical_name: "Compass",
  status: "provisional",
  body: { summary: "A brass compass." },
  extensions: {},
};

/** The finding the third-party app submits against its own entry. */
const SUBMITTED_FINDING: Finding = {
  schema_version: 1,
  finding_id: "demo-harbor/finding/compass-uncased",
  severity: "info",
  status: "open",
  title: "Compass uncased",
  description: "The compass has no case.",
  target_entry_id: SUBMITTED_ENTRY.entry_id,
  extensions: {},
};

/** Structural subset of `SpokeResult` — the client does not import operations. */
type AnySpokeResult<T> =
  | { ok: true; value: T }
  | { ok: false; code: string; message: string };

/** Unwrap a port-call result or fail the demo loudly (no silent fallbacks). */
function requireOk<T>(result: AnySpokeResult<T>): T {
  if (!result.ok) {
    throw new Error(
      `demo client: port call rejected (${result.code}): ${result.message}`,
    );
  }
  return result.value;
}

/** Everything the third-party flow produced, plus the teardown handle. */
export interface DemoClientRun {
  transport: WsTransport;
  adapter: RemoteAdapter;
  /** The server's manifest (session cache — spec D5, no round-trip). */
  serverManifest: HostCapabilityManifest;
  /** The server's derived peer_id. */
  remotePeerId: string;
  /** The entry as created (revision 1). */
  created: KnowledgeEntry;
  /** The entry after the compare-and-swap update (revision 2). */
  updated: KnowledgeEntry;
  /** The entry as fetched back after the update. */
  fetched: KnowledgeEntry;
  /** All knowledge entries in the demo namespace after the submission. */
  listed: KnowledgeEntry[];
  /** The stored findings (round-tripped). */
  findings: Finding[];
  /** The host's peer manifest list (empty — the demo host knows no peers). */
  peerManifests: HostCapabilityManifest[];
  /** Release the session + transport (idempotent). */
  close(): void;
}

/**
 * Execute the full third-party flow over a real WebSocket: dial, then
 * manifest → put (OCC create) → put (CAS update) → get → list → findings →
 * peer manifests. Every port call must succeed — a rejection throws.
 */
export async function runDemoClient(options: {
  url: string;
}): Promise<DemoClientRun> {
  const transport = new WsTransport(options.url);
  const adapter = await connectRemoteAdapter({
    transport,
    localIdentity: { seed: DEMO_CLIENT_SEED },
    localManifest: DEMO_CLIENT_MANIFEST,
    remotePubkey: DEMO_SERVER_PUBKEY,
    allowlist: [DEMO_SERVER_PEER_ID],
  });

  // Step 1 — capability manifest (cached at establish, no round-trip).
  const serverManifest = adapter.remoteManifest;

  // Step 2 — put → get round-trip with OCC: create, then compare-and-swap.
  const created = requireOk(
    await adapter.putKnowledgeEntry(SUBMITTED_ENTRY, null),
  );
  if (created.revision === undefined) {
    throw new Error("demo client: created entry has no revision");
  }
  const updated = requireOk(
    await adapter.putKnowledgeEntry(
      { ...SUBMITTED_ENTRY, status: "confirmed" },
      created.revision,
    ),
  );
  const fetched = requireOk(
    await adapter.getKnowledgeEntry(SUBMITTED_ENTRY.entry_id),
  );

  // Step 3 — list: seed corpus + submitted entry + engine-derived artifacts.
  const listed = requireOk(
    await adapter.listKnowledgeEntries({ scope_id: DEMO_SCOPE_ID }),
  );

  // Step 4 — findings round-trip.
  const findings = requireOk(await adapter.putFindings([SUBMITTED_FINDING]));

  // Step 5 — peer host manifests (the demo host knows no peers).
  const peerManifests = requireOk(
    await adapter.listPeerHostCapabilityManifests(),
  );

  return {
    transport,
    adapter,
    serverManifest,
    remotePeerId: adapter.remotePeerId,
    created,
    updated,
    fetched,
    listed,
    findings,
    peerManifests,
    close(): void {
      adapter.close();
      transport.close();
    },
  };
}

// ── CLI ────────────────────────────────────────────────────────────────────

function parseUrl(argv: string[]): string {
  const flagIndex = argv.indexOf("--url");
  if (flagIndex === -1) {
    return "ws://127.0.0.1:8787";
  }
  const raw = argv[flagIndex + 1];
  if (raw === undefined || raw.length === 0) {
    throw new Error("--url requires a value");
  }
  return raw;
}

async function main(): Promise<void> {
  const url = parseUrl(process.argv.slice(2));
  const run = await runDemoClient({ url });
  try {
    console.log("SPOKE connect demo — third-party client");
    console.log(`  dialing ${url} as ${DEMO_CLIENT_PEER_ID}`);
    console.log(
      `  remote peer: ${run.remotePeerId} (${run.serverManifest.host_id})`,
    );
    console.log(
      `    capabilities: ${run.serverManifest.capabilities.join(", ")}`,
    );
    console.log(`    namespaces:   ${run.serverManifest.namespaces.join(", ")}`);
    console.log(
      `  putKnowledgeEntry  ${run.created.entry_id} → revision ${run.created.revision}`,
    );
    console.log(
      `  putKnowledgeEntry  ${run.updated.entry_id} (CAS) → revision ${run.updated.revision}`,
    );
    console.log(
      `  getKnowledgeEntry  ${run.fetched.entry_id} → status ${run.fetched.status}`,
    );
    console.log(
      `  listKnowledgeEntries → ${run.listed.length} entries (${run.listed
        .map((entry) => entry.entry_id)
        .join(", ")})`,
    );
    console.log(
      `  putFindings        → ${run.findings.length} finding(s) stored`,
    );
    console.log("  listPeerHostCapabilityManifests → []");
    console.log("  done.");
  } finally {
    run.close();
  }
}

// Run the CLI only when executed directly (`node dist/main.js`), not when
// the e2e imports `runDemoClient` / `DEMO_CLIENT_MANIFEST` from this module.
const isCliEntry =
  process.argv[1] !== undefined &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isCliEntry) {
  main().catch((error) => {
    console.error(
      `demo client failed: ${error instanceof Error ? error.message : String(error)}`,
    );
    process.exitCode = 1;
  });
}
