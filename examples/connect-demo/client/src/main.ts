/**
 * The third-party client story (plan T3): `runDemoClient` dials a connect
 * host over a real WebSocket with the REAL library client
 * (`connectRemoteAdapter` from `@42ch/spoke-connect/remote`), executes the
 * demo flow through the drop-in async `BaselinePorts` surface plus the
 * optional `l2-computable` / `l5-fork` port faces (`project` / `compute` /
 * `listForkTimelineEvents`), and returns the asserted results. The CLI
 * (`node dist/main.js --url ws://…`) prints each story step.
 *
 * The client NEVER touches session-core verification helpers — it only
 * implements `Transport` and calls `connectRemoteAdapter` + the port
 * surfaces (spec D10 encapsulation).
 */

import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  ComputeResponse,
  ComputableFieldMap,
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  ProjectResponse,
  TimelineEvent,
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
import {
  LORE_LOOKUP_DESCRIPTOR,
  ROLL_DICE_DESCRIPTOR,
  TOY_WORLD_LORE_LOOKUP_ID,
  TOY_WORLD_ROLL_DICE_ID,
  loreLookup,
  rollDice,
} from "./tools/toy-world-tools.js";
import { WsTransport } from "./transport/ws-transport.js";

/**
 * The third-party app's own manifest (distinct from the server's). It
 * declares the two toy-world tools it can serve: the ids are frozen
 * (docs/snippet byte-parity), the `toy_world` namespace is owned here, and
 * `validateManifestTools` (spoke-operations) passes on this manifest — the
 * host discovers these tools from the authenticated manifest and
 * reverse-invokes them mid-orchestration. The optional `l2-computable` /
 * `l5-fork` families are declared too: the negotiated set is the
 * intersection of both manifests, so this client can drive the server's
 * optional port faces (and an undeclared server capability denies).
 */
export const DEMO_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: [
    "spoke-baseline",
    TOY_WORLD_ROLL_DICE_ID,
    TOY_WORLD_LORE_LOOKUP_ID,
    "l2-computable",
    "l5-fork",
  ],
  namespaces: [DEMO_SCOPE_ID, "toy_world"],
  tools: [ROLL_DICE_DESCRIPTOR, LORE_LOOKUP_DESCRIPTOR],
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

/**
 * The seeded l5-fork branch the client queries. Client-local copy — the
 * third-party client must not import the demo server package at runtime
 * (dep-surface story); the e2e catches any drift against the server's
 * seed corpus.
 */
export const DEMO_STORM_FORK_ID = "demo-harbor/fork/storm";

/** The l2-computable session the demo drives (project → compute settle). */
const COMPUTABLE_SESSION_ID = "demo-session/harbor-1";

/** The seeded harbor entry the computable flow targets. */
const COMPUTABLE_ENTRY_ID = "demo-harbor/location/harbor";

/** Static state the client projects into the session's computable view. */
const PROJECT_STATE: ComputableFieldMap = { ships_at_dock: 3 };

/** The computable delta applied on compute; settle merges it into static state. */
const COMPUTE_DELTA: ComputableFieldMap = { tide: "rising" };

/** Success branch of the ProjectResponse / ComputeResponse wire unions. */
type ProjectSuccess = Exclude<ProjectResponse, { error: unknown }>;
type ComputeSuccess = Exclude<ComputeResponse, { error: unknown }>;

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
  /** The tool ids this client registered on the dial (empty when not registered). */
  registeredToolIds: readonly string[];
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
  /**
   * l2-computable: the projected computable view (session materialized).
   * Present when the dialed manifest declared the optional families (the
   * default manifest does).
   */
  projected?: ProjectSuccess;
  /**
   * l2-computable: the settled computable view + derived static state.
   * Present when the dialed manifest declared the optional families.
   */
  computed?: ComputeSuccess;
  /**
   * l5-fork: the storm-fork timeline events (round-tripped). Present when
   * the dialed manifest declared the optional families.
   */
  forkEvents?: TimelineEvent[];
  /** Release the session + transport (idempotent). */
  close(): void;
}

export interface RunDemoClientOptions {
  url: string;
  /**
   * The manifest this client dials with. Defaults to
   * {@link DEMO_CLIENT_MANIFEST} (tools declared). The negative e2e dials
   * with a tools-less manifest to prove the host's reverse invoke is denied.
   */
  manifest?: HostCapabilityManifest;
  /**
   * Register the toy-world tool handlers on the dialed RemoteAdapter.
   * Defaults to true — the tool ids the client advertises must be servable.
   */
  registerTools?: boolean;
}

/**
 * Execute the full third-party flow over a real WebSocket: dial (registering
 * the toy-world tool handlers on the RemoteAdapter so the host can
 * reverse-invoke them), then manifest → put (OCC create) → put (CAS update)
 * → get → list → findings → peer manifests → the optional families:
 * l2-computable (project → compute settle → derived state) and l5-fork
 * (listForkTimelineEvents over the seeded storm fork). Every port call must
 * succeed — a rejection throws.
 */
export async function runDemoClient(
  options: RunDemoClientOptions,
): Promise<DemoClientRun> {
  const transport = new WsTransport(options.url);
  const dialManifest = options.manifest ?? DEMO_CLIENT_MANIFEST;
  const adapter = await connectRemoteAdapter({
    transport,
    localIdentity: { seed: DEMO_CLIENT_SEED },
    localManifest: dialManifest,
    remotePubkey: DEMO_SERVER_PUBKEY,
    allowlist: [DEMO_SERVER_PEER_ID],
  });

  // Step 0 — tool handlers: register the toy-world tools on the dial so the
  // host's mid-orchestration reverse invokes are served. `lore_lookup` reads
  // the client's own lore store (entries it submitted).
  const loreStore = new Map<string, KnowledgeEntry>();
  if (options.registerTools ?? true) {
    adapter.registerToolHandler(TOY_WORLD_ROLL_DICE_ID, rollDice);
    adapter.registerToolHandler(
      TOY_WORLD_LORE_LOOKUP_ID,
      loreLookup(loreStore),
    );
  }

  // Step 1 — capability manifest (cached at establish, no round-trip).
  const serverManifest = adapter.remoteManifest;

  // Step 2 — put → get round-trip with OCC: create, then compare-and-swap.
  const created = requireOk(
    await adapter.putKnowledgeEntry(SUBMITTED_ENTRY, null),
  );
  if (created.revision === undefined) {
    throw new Error("demo client: created entry has no revision");
  }
  loreStore.set(created.entry_id, created);
  const updated = requireOk(
    await adapter.putKnowledgeEntry(
      { ...SUBMITTED_ENTRY, status: "confirmed" },
      created.revision,
    ),
  );
  loreStore.set(updated.entry_id, updated);
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

  // Steps 6-7 — optional families: drive them only when THIS client's
  // manifest declares them (the negotiated set is the intersection of both
  // manifests, so a server that does not declare a family denies loudly
  // through requireOk instead of skipping silently). The default manifest
  // declares both, so the demo flow always runs them.
  const drivesOptionalOps =
    dialManifest.capabilities.includes("l2-computable") &&
    dialManifest.capabilities.includes("l5-fork");

  // Step 6 — l2-computable round-trip: project materializes the session's
  // computable view from static state; compute applies the delta and
  // settles it back into static state (the derived state).
  let projected: ProjectSuccess | undefined;
  let computed: ComputeSuccess | undefined;
  let forkEvents: TimelineEvent[] | undefined;
  if (drivesOptionalOps) {
    const projectedResult = requireOk(
      await adapter.project({
        session_id: COMPUTABLE_SESSION_ID,
        entry_id: COMPUTABLE_ENTRY_ID,
        state: { ...PROJECT_STATE },
      }),
    );
    if ("error" in projectedResult) {
      throw new Error(
        `demo client: project answered an error branch (${projectedResult.error.code})`,
      );
    }
    projected = projectedResult;

    const computedResult = requireOk(
      await adapter.compute({
        session_id: COMPUTABLE_SESSION_ID,
        entry_id: COMPUTABLE_ENTRY_ID,
        computable: { ...COMPUTE_DELTA },
        settle: true,
      }),
    );
    if ("error" in computedResult) {
      throw new Error(
        `demo client: compute answered an error branch (${computedResult.error.code})`,
      );
    }
    computed = computedResult;

    // Step 7 — l5-fork round-trip: the seeded storm-fork timeline.
    forkEvents = requireOk(
      await adapter.listForkTimelineEvents({
        scope_id: DEMO_SCOPE_ID,
        fork_id: DEMO_STORM_FORK_ID,
      }),
    );
  }

  return {
    transport,
    adapter,
    serverManifest,
    remotePeerId: adapter.remotePeerId,
    registeredToolIds:
      options.registerTools ?? true
        ? [TOY_WORLD_ROLL_DICE_ID, TOY_WORLD_LORE_LOOKUP_ID]
        : [],
    created,
    updated,
    fetched,
    listed,
    findings,
    peerManifests,
    projected,
    computed,
    forkEvents,
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
      `  tools registered: ${run.registeredToolIds.join(", ")} (served on reverse invoke)`,
    );
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
    if (run.projected !== undefined && run.computed !== undefined) {
      console.log(
        `  project            ${run.projected.entry_id} → ${JSON.stringify(run.projected.computable)}`,
      );
      console.log(
        `  compute (settle)   ${run.computed.entry_id} → ${JSON.stringify(run.computed.computable)} state ${JSON.stringify(run.computed.state)}`,
      );
    }
    if (run.forkEvents !== undefined) {
      console.log(
        `  listForkTimelineEvents → ${run.forkEvents.length} event(s) (${run.forkEvents
          .map((event) => event.timeline_event_id)
          .join(", ")})`,
      );
    }
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
