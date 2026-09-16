import type {
  ExtractRequest,
  ExtractResponse,
  KnowledgeEntry,
  SourceAnchor,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  SpokeRejectCode,
  orchestrateExtract,
  spokeOk,
  spokeReject,
  type ExtractionPort,
  type SpokeResult,
} from "../index.js";

type ExtractSuccess = Extract<ExtractResponse, { candidates: KnowledgeEntry[] }>;

function makeAnchor(overrides: Partial<SourceAnchor> = {}): SourceAnchor {
  return {
    schema_version: 1,
    source_id: "chapter-01",
    extensions: {},
    ...overrides,
  };
}

function makeCandidate(
  overrides: Partial<KnowledgeEntry> & Pick<KnowledgeEntry, "entry_id">,
): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_type: "character",
    canonical_name: "Mira Vale",
    status: "provisional",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

function makeRequest(overrides: Partial<ExtractRequest> = {}): ExtractRequest {
  return {
    run_id: "run_001",
    sources: [makeAnchor()],
    ...overrides,
  };
}

/** Port double that records the requests handed to the loader. */
function createExtractionPort(
  load: (request: ExtractRequest) => Promise<SpokeResult<unknown>>,
): ExtractionPort & { loadCalls: ExtractRequest[] } {
  const loadCalls: ExtractRequest[] = [];
  return {
    loadCalls,
    async loadExtractionInput(request) {
      loadCalls.push(request);
      return load(request);
    },
  };
}

/** Narrow the closed success branch so candidates/run are observable. */
function expectSuccess(result: SpokeResult<ExtractResponse>): ExtractSuccess {
  expect(result.ok).toBe(true);
  if (!result.ok || !("candidates" in result.value)) {
    throw new Error("expected an extraction success response");
  }
  return result.value;
}

describe("orchestrateExtract", () => {
  it("rejects malformed request boundaries with INVALID_INPUT before any port or extractor call", async () => {
    const invalidRequests: ExtractRequest[] = [
      makeRequest({ run_id: "" }),
      makeRequest({ run_id: 42 as unknown as string }),
      makeRequest({ sources: [] as unknown as ExtractRequest["sources"] }),
      makeRequest({
        sources: undefined as unknown as ExtractRequest["sources"],
      }),
    ];
    let extractorCalls = 0;

    for (const request of invalidRequests) {
      const port = createExtractionPort(async () => spokeOk({ loaded: true }));
      const result = await orchestrateExtract(port, request, async () => {
        extractorCalls += 1;
        return spokeOk({ candidates: [] });
      });

      expect(result.ok).toBe(false);
      if (result.ok) {
        continue;
      }
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(port.loadCalls).toEqual([]);
    }

    expect(extractorCalls).toBe(0);
  });

  it("rejects a missing loading method with CAPABILITY_PORT_MISSING for ke-extraction", async () => {
    const missingPorts: ExtractionPort[] = [
      {} as ExtractionPort,
      null as unknown as ExtractionPort,
    ];
    let extractorCalls = 0;

    for (const ports of missingPorts) {
      const result = await orchestrateExtract(ports, makeRequest(), async () => {
        extractorCalls += 1;
        return spokeOk({ candidates: [] });
      });

      expect(result.ok).toBe(false);
      if (result.ok) {
        continue;
      }
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details?.capability).toBe("ke-extraction");
    }

    expect(extractorCalls).toBe(0);
  });

  it("awaits the async extractor and returns its provisional candidates with the echoed run id", async () => {
    const request = makeRequest();
    const loadedInput = { chapter: "raw manuscript text" };
    const candidate = makeCandidate({ entry_id: "kb_extract_1" });
    const port = createExtractionPort(async () => spokeOk(loadedInput));
    let releaseExtractor!: () => void;
    const extractorGate = new Promise<void>((resolve) => {
      releaseExtractor = resolve;
    });
    let signalStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      signalStarted = resolve;
    });
    let settled = false;
    let extractorCalls = 0;

    const pending = orchestrateExtract(port, request, async (input) => {
      extractorCalls += 1;
      expect(input.request).toBe(request);
      expect(input.input).toBe(loadedInput);
      signalStarted();
      await extractorGate;
      return spokeOk({ candidates: [candidate] });
    }).then((result) => {
      settled = true;
      return result;
    });

    await started;
    // The callback is still blocked on its own await, so an orchestrator that
    // did not await it would already have settled by now.
    expect(settled).toBe(false);

    releaseExtractor();
    const response = expectSuccess(await pending);

    expect(extractorCalls).toBe(1);
    expect(port.loadCalls).toEqual([request]);
    expect(response.candidates).toEqual([candidate]);
    expect(response.candidates[0]).toBe(candidate);
    expect(response.candidates[0].status).toBe("provisional");
    expect(response.run).toStrictEqual({ run_id: "run_001" });
  });

  it("returns the load rejection unchanged and never invokes the extractor", async () => {
    const failure = spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "source store unavailable",
    );
    const port = createExtractionPort(async () => failure);
    let extractorCalls = 0;

    const result = await orchestrateExtract(port, makeRequest(), async () => {
      extractorCalls += 1;
      return spokeOk({ candidates: [] });
    });

    expect(result).toBe(failure);
    expect(extractorCalls).toBe(0);
  });

  it("returns the extractor rejection unchanged", async () => {
    const failure = spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "extraction backend down",
    );
    const port = createExtractionPort(async () => spokeOk({ chapter: "text" }));

    const result = await orchestrateExtract(
      port,
      makeRequest(),
      async () => failure,
    );

    expect(result).toBe(failure);
  });

  it("rejects the whole set when a candidate has a terminal status", async () => {
    const provisional = makeCandidate({ entry_id: "kb_ok" });
    const deleted = makeCandidate({ entry_id: "kb_deleted", status: "deleted" });
    const port = createExtractionPort(async () => spokeOk({}));

    const result = await orchestrateExtract(port, makeRequest(), async () =>
      spokeOk({ candidates: [provisional, deleted] }),
    );

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.CANDIDATE_TERMINAL_STATUS);
    expect(result.details?.status).toBe("deleted");
    expect(result.details?.entry_id).toBe("kb_deleted");
  });

  it("rejects the whole set when a candidate is not provisional", async () => {
    const provisional = makeCandidate({ entry_id: "kb_ok" });
    const confirmed = makeCandidate({
      entry_id: "kb_confirmed",
      status: "confirmed",
    });
    const port = createExtractionPort(async () => spokeOk({}));

    const result = await orchestrateExtract(port, makeRequest(), async () =>
      spokeOk({ candidates: [provisional, confirmed] }),
    );

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.CANDIDATE_NOT_PROVISIONAL);
    expect(result.details?.status).toBe("confirmed");
  });

  it("returns a successful empty candidate set when the extractor finds nothing", async () => {
    const port = createExtractionPort(async () => spokeOk({ chapter: "text" }));

    const result = await orchestrateExtract(port, makeRequest(), async () =>
      spokeOk({ candidates: [] }),
    );

    const response = expectSuccess(result);
    expect(response.candidates).toEqual([]);
    expect(response.run).toStrictEqual({ run_id: "run_001" });
  });

  it("echoes the caller run id and retains opaque advisory metadata verbatim", async () => {
    const coverageHint = ["chapter-01", { ratio: 0.5 }];
    const request = makeRequest({ run_id: "run_opaque" });
    const candidate = makeCandidate({ entry_id: "kb_opaque" });
    const port = createExtractionPort(async () => spokeOk({ chapter: "text" }));

    const result = await orchestrateExtract(port, request, async () =>
      spokeOk({
        candidates: [candidate],
        method: "rule-based-v1",
        coverage_hint: coverageHint,
      }),
    );

    const response = expectSuccess(result);
    expect(response.run).toStrictEqual({
      run_id: "run_opaque",
      method: "rule-based-v1",
      coverage_hint: coverageHint,
    });
    expect(response.run.coverage_hint).toBe(coverageHint);
  });

  it("emits no method key when the extractor returns an empty method", async () => {
    const request = makeRequest({ run_id: "run_method_empty" });
    const port = createExtractionPort(async () => spokeOk({ chapter: "text" }));

    const result = await orchestrateExtract(port, request, async () =>
      spokeOk({ candidates: [], method: "" }),
    );

    const response = expectSuccess(result);
    expect("method" in response.run).toBe(false);
    expect(response.run).toStrictEqual({ run_id: "run_method_empty" });
  });

  it("retains the shortest non-empty method verbatim", async () => {
    const port = createExtractionPort(async () => spokeOk({ chapter: "text" }));

    const result = await orchestrateExtract(port, makeRequest(), async () =>
      spokeOk({ candidates: [], method: "x" }),
    );

    const response = expectSuccess(result);
    expect(response.run).toStrictEqual({ run_id: "run_001", method: "x" });
  });

  it("treats an absent and a null coverage hint alike as no hint", async () => {
    const port = createExtractionPort(async () => spokeOk({ chapter: "text" }));

    const nothingFound = expectSuccess(
      await orchestrateExtract(port, makeRequest(), async () =>
        spokeOk({ candidates: [] }),
      ),
    );
    const nullHint = expectSuccess(
      await orchestrateExtract(port, makeRequest(), async () =>
        spokeOk({ candidates: [], coverage_hint: null }),
      ),
    );

    expect("coverage_hint" in nothingFound.run).toBe(false);
    expect(nullHint.run.coverage_hint).toBeNull();
  });
});
