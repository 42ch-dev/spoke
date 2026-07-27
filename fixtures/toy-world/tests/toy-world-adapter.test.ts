import { readFileSync } from "node:fs";
import { join } from "node:path";

import type {
  AssembleRequest,
  CheckRequest,
  ComputeRequest,
  ComputeResponse,
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  ProjectRequest,
  ProjectResponse,
  PromoteRequest,
  RelateRequest,
  Relation,
  Rule,
  TimelineEvent,
  UpsertRequest,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  orchestrateAssemble,
  orchestrateCheck,
  orchestrateCompute,
  orchestrateProject,
  orchestratePromote,
  orchestrateRelate,
  orchestrateUpsert,
  spokeOk,
  type CheckRunInput,
  type ComputablePorts,
  type FullAdapter,
} from "@42ch/spoke-operations";
import { describe, expect, it } from "vitest";

import {
  ToyWorldAdapter,
  asBaselineOnly,
} from "../src/adapter/index.js";
import { FIXTURES_ROOT } from "./schema-validator.js";

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

function provisionalMira(overrides: Partial<KnowledgeEntry> = {}): KnowledgeEntry {
  const mira = loadFixture<KnowledgeEntry>("kb_tw_mira.json");
  return {
    ...mira,
    status: "provisional",
    revision: 0,
    ...overrides,
  };
}

describe("ToyWorldAdapter baseline orchestration", () => {
  it("orchestrateUpsert creates a KnowledgeEntry through OCC put", () => {
    const adapter = new ToyWorldAdapter();
    const candidate: KnowledgeEntry = {
      schema_version: 1,
      entry_id: "kb_tw_new_cartographer",
      entry_type: "character",
      canonical_name: "New Cartographer",
      status: "provisional",
      body: { summary: "Fresh provisional entry" },
      extensions: {},
    };
    const request: UpsertRequest = { knowledge_entries: [candidate] };

    const result = orchestrateUpsert(adapter, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.knowledge_entries).toEqual([candidate]);
    expect(adapter.store.entries.get("kb_tw_new_cartographer")).toEqual(
      candidate,
    );
  });

  it("orchestratePromote persists a confirmed KnowledgeEntry", () => {
    const candidate = provisionalMira({
      entry_id: "kb_tw_promote",
      canonical_name: "Promote Candidate",
    });
    const adapter = new ToyWorldAdapter();
    const request: PromoteRequest = { candidate };

    const result = orchestratePromote(adapter, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.knowledge_entry.status).toBe("confirmed");
    expect(result.value.knowledge_entry.revision).toBe(1);
    expect(adapter.store.entries.get("kb_tw_promote")?.status).toBe(
      "confirmed",
    );
  });

  it("orchestrateRelate persists a Relation", () => {
    const adapter = new ToyWorldAdapter();
    const relation = loadFixture<Relation>("rel_tw_mira_harbor.json");
    const request: RelateRequest = {
      relation: {
        ...relation,
        relation_id: "rel_tw_adapter_demo",
      },
    };

    const result = orchestrateRelate(adapter, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.relation.relation_id).toBe("rel_tw_adapter_demo");
    expect(adapter.store.relations.get("rel_tw_adapter_demo")).toEqual(
      request.relation,
    );
  });

  it("orchestrateCheck runs a trivial checker and puts findings", () => {
    const mira = loadFixture<KnowledgeEntry>("kb_tw_mira.json");
    const harbor = loadFixture<KnowledgeEntry>("kb_tw_harbor.json");
    const rule = loadFixture<Rule>("rule_tw_consistency.json");
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const request: CheckRequest = {
      scope: {
        scope_id: "toy-scope-001",
        entry_ids: ["kb_tw_mira", "kb_tw_harbor"],
      },
      rule_refs: ["rule_tw_consistency"],
    };
    const finding = loadFixture<Finding>("fnd_tw_open.json");

    const result = orchestrateCheck(
      adapter,
      request,
      (input: CheckRunInput) => {
        expect(input.entries.map((e) => e.entry_id).sort()).toEqual([
          "kb_tw_harbor",
          "kb_tw_mira",
        ]);
        expect(input.rules).toEqual([rule]);
        expect(input.entries).toEqual(
          expect.arrayContaining([mira, harbor]),
        );
        return spokeOk([
          {
            ...finding,
            finding_id: "fnd_tw_adapter_check",
          },
        ]);
      },
    );

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.findings).toHaveLength(1);
    expect(result.value.findings[0]?.finding_id).toBe("fnd_tw_adapter_check");
    expect(
      adapter.store.findings.some((f) => f.finding_id === "fnd_tw_adapter_check"),
    ).toBe(true);
  });

  it("orchestrateAssemble builds a packet from scoped KnowledgeEntries", () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const request: AssembleRequest = {
      scope: {
        scope_id: "toy-scope-001",
        entry_ids: ["kb_tw_mira", "kb_tw_harbor"],
      },
    };

    const result = orchestrateAssemble(adapter, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet.packet_id).toBe("assemble:toy-scope-001");
    expect(result.value.packet.entries).toHaveLength(2);
    expect(result.value.packet.entries.map((e) => e.entry_id).sort()).toEqual([
      "kb_tw_harbor",
      "kb_tw_mira",
    ]);
  });

  it("putKnowledgeEntry rejects OCC mismatch (STORED_REVISION_STALE)", () => {
    const stored = provisionalMira({
      entry_id: "kb_tw_occ",
      revision: 2,
    });
    const adapter = new ToyWorldAdapter({ entries: [stored] });
    const updated: KnowledgeEntry = {
      ...stored,
      canonical_name: "Stale write",
      revision: 3,
    };

    const result = adapter.putKnowledgeEntry(updated, 1);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
    }
  });

  it("returns CAPABILITY_PORT_MISSING for baseline-only adapter at dynamic boundary", () => {
    const full = new ToyWorldAdapter();
    const baseline = asBaselineOnly(full);
    const ports = baseline as unknown as ComputablePorts;
    const request: ProjectRequest = loadFixture<ProjectRequest>(
      "op_tw_project_request.json",
    );

    const result = orchestrateProject(ports, request);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
    }
  });
});

describe("ToyWorldAdapter HostManifestPort", () => {
  it("returns the primary host manifest from getHostCapabilityManifest", () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const primary = loadFixture<HostCapabilityManifest>("host_tw_primary.json");

    const result = adapter.getHostCapabilityManifest();

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual(primary);
    expect(result.value.host_id).toBe("host_tw_primary");
    expect(result.value.roles).toContain("assembler");
    expect(result.value.roles).toContain("data-store");
  });

  it("lists peer manifests excluding self with ascending host_id sort", () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const primary = loadFixture<HostCapabilityManifest>("host_tw_primary.json");
    const peer = loadFixture<HostCapabilityManifest>("host_tw_peer.json");

    const result = adapter.listPeerHostCapabilityManifests();

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual([peer]);
    expect(result.value.map((manifest) => manifest.host_id)).toEqual([
      "host_tw_peer",
    ]);
    expect(result.value[0]?.host_id).not.toBe(primary.host_id);

    const primaryNamespaces = new Set(primary.namespaces);
    const peerNamespaces = new Set(peer.namespaces);
    const overlap = [...primaryNamespaces].filter((ns) =>
      peerNamespaces.has(ns),
    );
    expect(overlap).toEqual([]);
  });
});

describe("ToyWorldAdapter FullAdapter optional ports", () => {
  it("satisfies FullAdapter and orchestrateProject returns fixture-shaped success", () => {
    const adapter: FullAdapter = ToyWorldAdapter.withCommittedFixtures();
    const request = loadFixture<ProjectRequest>("op_tw_project_request.json");
    const fixtureResponse = loadFixture<ProjectResponse>(
      "op_tw_project_response.json",
    );

    const result = orchestrateProject(adapter, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual({
      session_id: request.session_id,
      entry_id: request.entry_id,
      computable: fixtureResponse.computable,
    });
  });

  it("orchestrateCompute settle returns wire-valid success from settle response fixture", () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const request = loadFixture<ComputeRequest>(
      "op_tw_compute_settle_request.json",
    );
    const fixtureResponse = loadFixture<ComputeResponse>(
      "op_tw_compute_settle_response.json",
    );

    const result = orchestrateCompute(adapter, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.session_id).toBe(request.session_id);
    expect(result.value.entry_id).toBe(request.entry_id);
    expect(result.value.computable).toEqual(request.computable);
    expect(result.value.state).toEqual(fixtureResponse.state);
  });

  it("listForkTimelineEvents returns seeded events for fork_tw_storm_branch", () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const storm = loadFixture<TimelineEvent>("evt_tw_harbor_storm_delay.json");

    const result = adapter.listForkTimelineEvents({
      scope_id: "toy-scope-001",
      fork_id: "fork_tw_storm_branch",
    });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual([storm]);
  });
});
