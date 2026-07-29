import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { Ajv, type ValidateFunction } from "ajv";
import * as ajvFormatsModule from "ajv-formats";

type RegisterFormats = (ajv: Ajv) => Ajv;

const addFormats = ajvFormatsModule.default as unknown as RegisterFormats;

const REPO_ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "../../..");
const SCHEMAS_ROOT = join(REPO_ROOT, "schemas");

function collectSchemaFiles(dir: string): string[] {
  const entries = readdirSync(dir);
  const files: string[] = [];

  for (const entry of entries) {
    const fullPath = join(dir, entry);
    const stats = statSync(fullPath);

    if (stats.isDirectory()) {
      files.push(...collectSchemaFiles(fullPath));
      continue;
    }

    if (entry.endsWith(".schema.json")) {
      files.push(fullPath);
    }
  }

  return files;
}

export function createSchemaValidator(): Ajv {
  const ajv = new Ajv({
    allErrors: true,
    strict: false,
    validateSchema: false,
  });
  addFormats(ajv);

  for (const schemaPath of collectSchemaFiles(SCHEMAS_ROOT)) {
    const schema = JSON.parse(readFileSync(schemaPath, "utf8")) as {
      $id?: string;
    };

    if (typeof schema.$id === "string") {
      ajv.addSchema(schema, schema.$id);
    }
  }

  return ajv;
}

export function compileSchemaValidator(
  ajv: Ajv,
  schemaId: string,
): ValidateFunction {
  const validate = ajv.getSchema(schemaId);

  if (validate === undefined) {
    throw new Error(`Schema not registered: ${schemaId}`);
  }

  return validate;
}

export const SCHEMA_IDS = {
  knowledgeEntry: "https://spoke42.invalid/schemas/data/knowledge-entry.schema.json",
  relation: "https://spoke42.invalid/schemas/data/relation.schema.json",
  sourceAnchor: "https://spoke42.invalid/schemas/data/source-anchor.schema.json",
  timelineEvent: "https://spoke42.invalid/schemas/data/timeline-event.schema.json",
  rule: "https://spoke42.invalid/schemas/data/rule.schema.json",
  finding: "https://spoke42.invalid/schemas/data/finding.schema.json",
  assemblePacket: "https://spoke42.invalid/schemas/data/assemble-packet.schema.json",
  checkRequest: "https://spoke42.invalid/schemas/ops/check-request.schema.json",
  projectRequest: "https://spoke42.invalid/schemas/ops/project-request.schema.json",
  projectResponse: "https://spoke42.invalid/schemas/ops/project-response.schema.json",
  computeRequest: "https://spoke42.invalid/schemas/ops/compute-request.schema.json",
  computeResponse: "https://spoke42.invalid/schemas/ops/compute-response.schema.json",
  hostCapabilityManifest:
    "https://spoke42.invalid/schemas/data/host-capability-manifest.schema.json",
} as const;

export const FIXTURE_SCHEMA_MAP: Record<string, string> = {
  "kb_tw_mira.json": SCHEMA_IDS.knowledgeEntry,
  "kb_tw_harbor.json": SCHEMA_IDS.knowledgeEntry,
  "kb_tw_harbor_dawn_event.json": SCHEMA_IDS.knowledgeEntry,
  "kb_tw_harbor_market_square_event.json": SCHEMA_IDS.knowledgeEntry,
  "kb_tw_harbor_customs_gate_beat.json": SCHEMA_IDS.knowledgeEntry,
  "kb_tw_harbor_berth_confirm_event.json": SCHEMA_IDS.knowledgeEntry,
  "anchor_tw_manuscript.json": SCHEMA_IDS.sourceAnchor,
  "rel_tw_mira_harbor.json": SCHEMA_IDS.relation,
  "rel_tw_harbor_precedes_dawn_to_market.json": SCHEMA_IDS.relation,
  "rel_tw_harbor_precedes_market_to_customs.json": SCHEMA_IDS.relation,
  "rel_tw_harbor_precedes_customs_to_berth.json": SCHEMA_IDS.relation,
  "evt_tw_harbor_dawn.json": SCHEMA_IDS.timelineEvent,
  "evt_tw_harbor_market_square.json": SCHEMA_IDS.timelineEvent,
  "evt_tw_harbor_customs_gate.json": SCHEMA_IDS.timelineEvent,
  "evt_tw_harbor_berth_confirm.json": SCHEMA_IDS.timelineEvent,
  "evt_tw_harbor_storm_delay.json": SCHEMA_IDS.timelineEvent,
  "rule_tw_consistency.json": SCHEMA_IDS.rule,
  "fnd_tw_open.json": SCHEMA_IDS.finding,
  "pkt_tw_scope.json": SCHEMA_IDS.assemblePacket,
  "op_tw_check_request.json": SCHEMA_IDS.checkRequest,
  "op_tw_project_request.json": SCHEMA_IDS.projectRequest,
  "op_tw_project_response.json": SCHEMA_IDS.projectResponse,
  "op_tw_compute_request.json": SCHEMA_IDS.computeRequest,
  "op_tw_compute_settle_request.json": SCHEMA_IDS.computeRequest,
  "op_tw_compute_settle_response.json": SCHEMA_IDS.computeResponse,
  "host_tw_primary.json": SCHEMA_IDS.hostCapabilityManifest,
  "host_tw_peer.json": SCHEMA_IDS.hostCapabilityManifest,
};

export const FIXTURES_ROOT = join(REPO_ROOT, "fixtures/toy-world");
