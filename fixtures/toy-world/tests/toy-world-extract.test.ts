import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  SpokeRejectCode,
  orchestrateExtract,
  spokeOk,
  toErrorEnvelope,
  type ExtractionPort,
  type SpokeResult,
} from "@42ch/spoke-operations";
import type {
  ExtractRequest,
  ExtractResponse,
  HostCapabilityManifest,
  KnowledgeEntry,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  FIXTURE_SCHEMA_MAP,
  FIXTURES_ROOT,
  compileSchemaValidator,
  createSchemaValidator,
} from "./schema-validator.js";

const REQUEST_FIXTURE = "op_tw_extract_request.json";
const SUCCESS_FIXTURE = "op_tw_extract_response.json";
const ERROR_FIXTURE = "op_tw_extract_error_response.json";
const HOST_FIXTURE = "host_tw_extractor.json";

type ExtractSuccess = Extract<ExtractResponse, { candidates: KnowledgeEntry[] }>;
type ExtractFailure = Extract<ExtractResponse, { error: unknown }>;

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

/**
 * Host-local extractor rule standing in for a product extraction service: one
 * provisional `info_point` per referenced source, named from the anchor where it
 * carries a label, and keyed back to the anchor it came from.
 */
function proposeCandidates(request: ExtractRequest): KnowledgeEntry[] {
  return request.sources.map((anchor, index) => {
    // An absent span means the referenced artifact as a whole (ADR §1.2).
    const range =
      anchor.span === undefined
        ? anchor.source_id
        : `${anchor.source_id} offsets ${anchor.span.start}-${anchor.span.end}`;

    return {
      schema_version: 1,
      entry_id: `kb_tw_extract_${index + 1}`,
      entry_type: "info_point",
      canonical_name: anchor.label ?? `Extracted note ${index + 1}`,
      status: "provisional",
      body: { summary: `Provisional candidate proposed from ${range}.` },
      source_anchor: anchor,
      extensions: {},
    };
  });
}

/** Narrow the closed success branch so candidates and run are observable. */
function expectSuccess(result: SpokeResult<ExtractResponse>): ExtractSuccess {
  if (!result.ok || !("candidates" in result.value)) {
    throw new Error(
      `expected an extraction success response, got ${JSON.stringify(result)}`,
    );
  }
  return result.value;
}

describe("fixtures/toy-world extraction wire", () => {
  const ajv = createSchemaValidator();
  const request = loadFixture<ExtractRequest>(REQUEST_FIXTURE);
  const success = loadFixture<ExtractSuccess>(SUCCESS_FIXTURE);
  const failure = loadFixture<ExtractFailure>(ERROR_FIXTURE);
  const host = loadFixture<HostCapabilityManifest>(HOST_FIXTURE);

  it("validates the four portable extraction samples with the AJV loader", () => {
    const sampleFiles = [
      REQUEST_FIXTURE,
      SUCCESS_FIXTURE,
      ERROR_FIXTURE,
      HOST_FIXTURE,
    ];

    for (const filename of sampleFiles) {
      const schemaId = FIXTURE_SCHEMA_MAP[filename];

      expect(schemaId, `missing schema mapping for ${filename}`).toBeDefined();

      const validate = compileSchemaValidator(ajv, schemaId!);
      const valid = validate(loadFixture<unknown>(filename));

      expect(validate.errors, JSON.stringify(validate.errors, null, 2)).toBeNull();
      expect(valid).toBe(true);
    }
  });

  it("declares an input-source host claiming only the ke-extraction capability", () => {
    expect(host.host_id).toBe("host_tw_extractor");
    // ADR §1.1: the existing input-source role plus the optional ke-extraction
    // flag — no new role, no baseline claim, no ownership coupling.
    expect(host.roles).toEqual(["input-source"]);
    expect(host.capabilities).toEqual(["ke-extraction"]);
    expect(host.namespaces).toEqual(["extract_demo"]);
  });

  it("reproduces the committed success sample through orchestrateExtract", async () => {
    // Host-local source content: loaded in-process by the port, never on the wire.
    const loadedInput = {
      source_id: "manuscript:tw-ch1",
      segments: [
        { start: 48, end: 312, text: "Bells over the harbor at dawn." },
        { start: 312, end: 420, text: "The customs gate opened late." },
      ],
    };
    const loadCalls: ExtractRequest[] = [];
    const ports: ExtractionPort = {
      async loadExtractionInput(loadedRequest) {
        loadCalls.push(loadedRequest);
        return spokeOk(loadedInput);
      },
    };
    let extractorCalls = 0;
    let seenInput: unknown;

    const result = await orchestrateExtract(ports, request, async (input) => {
      extractorCalls += 1;
      seenInput = input.input;

      const candidates = proposeCandidates(input.request);

      return spokeOk({
        candidates,
        method: "toy-world-span-scan",
        coverage_hint: {
          sources_read: input.request.sources.length,
          candidates_proposed: candidates.length,
        },
      });
    });

    const response = expectSuccess(result);

    // Correlation: the response sample echoes the request fixture's run id.
    expect(success.run.run_id).toBe(request.run_id);
    // The committed wire sample is exactly what the host-local doubles produce.
    expect(response).toEqual(success);
    // Consumer-observable invariant (ADR §1.3): every candidate stays provisional.
    expect(
      response.candidates.every((candidate) => candidate.status === "provisional"),
    ).toBe(true);
    // One load, one extractor call, and the opaque host-local input unchanged.
    expect(loadCalls).toEqual([request]);
    expect(extractorCalls).toBe(1);
    expect(seenInput).toBe(loadedInput);
  });

  it("returns the committed error sample when a candidate is not provisional", async () => {
    const ports: ExtractionPort = {
      async loadExtractionInput() {
        return spokeOk({ source_id: "manuscript:tw-ch1" });
      },
    };

    const result = await orchestrateExtract(ports, request, async () =>
      spokeOk({
        candidates: proposeCandidates(request).map((candidate, index) =>
          index === 1 ? { ...candidate, status: "confirmed" } : candidate,
        ),
      }),
    );

    if (result.ok) {
      throw new Error("expected a non-provisional candidate rejection");
    }

    expect(result.code).toBe(SpokeRejectCode.CANDIDATE_NOT_PROVISIONAL);

    // The committed error sample is the library's own wire shape for this
    // failure, converted at the documented toErrorEnvelope boundary.
    const wire: ExtractResponse = { error: toErrorEnvelope(result) };

    expect(wire).toEqual(failure);
  });

  it("rejects a sample that mixes the success and error branches", () => {
    const validate = compileSchemaValidator(
      ajv,
      FIXTURE_SCHEMA_MAP[SUCCESS_FIXTURE]!,
    );
    const mixed = { ...success, error: failure.error };

    expect(validate(success)).toBe(true);
    expect(validate(mixed)).toBe(false);
    expect(validate.errors?.some((error) => error.keyword === "oneOf")).toBe(true);
  });

  it("rejects an extract request carrying inline source text", () => {
    const validate = compileSchemaValidator(
      ajv,
      FIXTURE_SCHEMA_MAP[REQUEST_FIXTURE]!,
    );

    expect(validate(request)).toBe(true);

    // ADR §1.2: no inline-text field, even for small fragments — products
    // expose an ephemeral SourceAnchor locator instead.
    for (const inlineField of ["content", "text"]) {
      const inline = { ...request, [inlineField]: "Bells over the harbor at dawn." };

      expect(validate(inline)).toBe(false);
      expect(
        validate.errors?.some((error) => error.keyword === "additionalProperties"),
      ).toBe(true);
    }
  });
});
