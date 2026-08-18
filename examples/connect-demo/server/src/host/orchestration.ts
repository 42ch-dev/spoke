/**
 * DemoOrchestrator — the demo host's tool-assisted orchestration step
 * (Task 2 of tools-proofs-demo-docs).
 *
 * A `BaselinePorts` adapter wrapping the MockAdapter. When the client
 * submits its compass knowledge entry (create), the host runs a
 * deterministic orchestration step:
 *
 *   1. Discovery — validate + list the authenticated client manifest's
 *      `tools[]` (`responder.remoteManifest`; the manifest was verified at
 *      handshake, so this is the authenticated view).
 *   2. Reverse invoke — ask the client to roll 2d6 via
 *      `responder.invokeTool(tools.toy_world.roll_dice, { count: 2, sides: 6 })`.
 *      The client's registered RemoteAdapter handler serves it; an
 *      unlisted / not-negotiated tool is denied by the protocol
 *      (`op_unsupported` → `CAPABILITY_PORT_MISSING`) and the deny is
 *      recorded — the host never succeeds silently.
 *   3. Feed — record the roll result as a knowledge entry in the engine (a
 *      BaselinePorts orchestration step) so the client sees it on its next
 *      `listKnowledgeEntries`.
 *
 * Every orchestration run is appended to the shared records array the
 * server handle exposes, so the e2e can assert discovery, the reverse
 * invoke result, and the deny path.
 */

import type {
  ComputeRequest,
  ComputeResponse,
  Finding,
  ForkId,
  HostCapabilityManifest,
  KnowledgeEntry,
  ProjectRequest,
  ProjectResponse,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import {
  listTools,
  validateManifestTools,
  type FullPorts,
  type SpokeResult,
} from "@42ch/spoke-operations";
import type { ConnectResponder } from "@42ch/spoke-connect/remote";

import { MockAdapter } from "../adapter/mock-adapter.js";
import {
  TOY_WORLD_ROLL_DICE_ID,
} from "../tools/toy-world-tools.js";

/** The engine entry the orchestration's roll result feeds (BaselinePorts step). */
export const DICE_ROLL_ENTRY_ID = "demo-harbor/artifact/dice-roll";

/** The client submission that triggers the orchestration (the demo's compass). */
export const DICE_ROLL_TRIGGER_ENTRY_ID = "demo-harbor/item/compass";

/** The deterministic reverse-invoke the orchestration performs (2d6). */
export const ORCHESTRATION_ROLL_ARGS = { count: 2, sides: 6 } as const;

/** One recorded orchestration run (discovery → reverse invoke → feed). */
export interface DemoOrchestration {
  /** Tool ids discovered in the authenticated client manifest (manifest order). */
  discovered: string[];
  /** Whether the client manifest passed `validateManifestTools`. */
  manifestValid: boolean;
  /** The tool the host reverse-invoked mid-orchestration. */
  tool_id: string;
  /** The arguments the host passed to the reverse invoke. */
  args: Record<string, unknown>;
  /** The reverse-invoke result (a deny is recorded, never swallowed). */
  result:
    | { ok: true; value: unknown }
    | {
        ok: false;
        code: string;
        message: string;
        details?: Record<string, unknown>;
      };
  /** The engine entry the roll result fed (present only on success). */
  fed_entry_id?: string;
}

/** The roll-shaped success value of `tools.toy_world.roll_dice`. */
interface RollResult {
  rolls: number[];
  total: number;
}

function diceRollEntry(roll: RollResult): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_id: DICE_ROLL_ENTRY_ID,
    entry_type: "note",
    canonical_name: "Dice roll",
    status: "confirmed",
    body: {
      summary: `Orchestration dice roll (2d6) → total ${roll.total}`,
      computable: { rolls: roll.rolls, total: roll.total },
    },
    extensions: {},
  };
}

/**
 * FullPorts adapter with the tool-assisted orchestration step. All port
 * families delegate to the wrapped MockAdapter except `putKnowledgeEntry`,
 * which runs the orchestration after the client's compass submission lands.
 * The optional `l2-computable` / `l5-fork` families delegate too — the
 * injected ports object serves them through the responder's structural
 * probe (gate → probe → serve/deny).
 */
export class DemoOrchestrator implements FullPorts {
  readonly #adapter: MockAdapter;
  readonly #records: DemoOrchestration[];
  #responder: ConnectResponder | null = null;

  constructor(adapter: MockAdapter, records: DemoOrchestration[]) {
    this.#adapter = adapter;
    this.#records = records;
  }

  /** Bind the established responder (late-bound — the factory is async). */
  setResponder(responder: ConnectResponder): void {
    this.#responder = responder;
  }

  async getKnowledgeEntry(entryId: string): Promise<SpokeResult<KnowledgeEntry>> {
    return this.#adapter.getKnowledgeEntry(entryId);
  }

  async putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>> {
    const result = await this.#adapter.putKnowledgeEntry(
      entry,
      expectedBaseRevision,
    );
    // Mid-flow orchestration: after the client's compass submission lands
    // (create), run discovery → reverse invoke → feed before answering.
    if (
      result.ok &&
      entry.entry_id === DICE_ROLL_TRIGGER_ENTRY_ID &&
      expectedBaseRevision === null
    ) {
      await this.#runOrchestration();
    }
    return result;
  }

  async getRelation(relationId: string): Promise<SpokeResult<Relation>> {
    return this.#adapter.getRelation(relationId);
  }

  async putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<Relation>> {
    return this.#adapter.putRelation(relation, expectedBaseRevision);
  }

  async listKnowledgeEntries(scope: Scope): Promise<SpokeResult<KnowledgeEntry[]>> {
    return this.#adapter.listKnowledgeEntries(scope);
  }

  async listTimelineEvents(scope: Scope): Promise<SpokeResult<TimelineEvent[]>> {
    return this.#adapter.listTimelineEvents(scope);
  }

  async project(request: ProjectRequest): Promise<SpokeResult<ProjectResponse>> {
    return this.#adapter.project(request);
  }

  async compute(request: ComputeRequest): Promise<SpokeResult<ComputeResponse>> {
    return this.#adapter.compute(request);
  }

  async listForkTimelineEvents(
    scope: Scope & { fork_id: ForkId },
  ): Promise<SpokeResult<TimelineEvent[]>> {
    return this.#adapter.listForkTimelineEvents(scope);
  }

  async putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>> {
    return this.#adapter.putFindings(findings);
  }

  async listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>> {
    return this.#adapter.listRules(ruleRefs);
  }

  async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
    return this.#adapter.getHostCapabilityManifest();
  }

  async listPeerHostCapabilityManifests(): Promise<
    SpokeResult<HostCapabilityManifest[]>
  > {
    return this.#adapter.listPeerHostCapabilityManifests();
  }

  // ── orchestration ───────────────────────────────────────────────────────

  async #runOrchestration(): Promise<void> {
    const responder = this.#responder;
    if (responder === null || responder.state !== "Established") {
      // Defensive-only: through the real transport the responder binds
      // before the handshake completes and invokes dispatch only while
      // Established, so this is unreachable in practice. Record the skip
      // anyway instead of a silent return, so every orchestration trigger
      // leaves an audit trace.
      this.#records.push({
        discovered: [],
        manifestValid: false,
        tool_id: TOY_WORLD_ROLL_DICE_ID,
        args: { ...ORCHESTRATION_ROLL_ARGS },
        result: {
          ok: false,
          code: "SKIPPED",
          message: "orchestration skipped: no established responder",
          details: { reason: "responder_unavailable" },
        },
      });
      return;
    }

    // 1. Discovery — the authenticated manifest (verified at handshake).
    const manifest: HostCapabilityManifest = responder.remoteManifest;
    const validated = validateManifestTools(manifest);
    const discovered = listTools(manifest).map(
      (descriptor) => descriptor.capability_id,
    );

    // 2. Reverse invoke — the protocol denies an unlisted / not-negotiated
    // tool; the deny is recorded, never swallowed.
    const result = await responder.invokeTool(
      TOY_WORLD_ROLL_DICE_ID,
      { ...ORCHESTRATION_ROLL_ARGS },
    );
    const record: DemoOrchestration = {
      discovered,
      manifestValid: validated.ok,
      tool_id: TOY_WORLD_ROLL_DICE_ID,
      args: { ...ORCHESTRATION_ROLL_ARGS },
      result: result.ok
        ? { ok: true, value: result.value }
        : {
            ok: false,
            code: result.code,
            message: result.message,
            ...(result.details !== undefined
              ? { details: result.details }
              : {}),
          },
    };

    // 3. Feed — the roll result lands in the engine as a knowledge entry
    // (a BaselinePorts orchestration step the client sees on its next list).
    if (result.ok) {
      const roll = result.value as RollResult;
      const feed = this.#adapter.engine.putKnowledgeEntry(
        diceRollEntry(roll),
        null,
      );
      if (feed.ok) {
        record.fed_entry_id = feed.value.entry_id;
      }
    }

    this.#records.push(record);
  }
}
