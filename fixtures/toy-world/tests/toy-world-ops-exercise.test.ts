import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  assertRevisionMatch,
  buildAssemblePacket,
  filterKnowledgeEntriesByScope,
  knowledgeEntryMatchesScope,
  timelineEventMatchesScope,
  transitionFindingStatus,
  validateComputableLogEntry,
  validateComputeRequest,
  validateProjectRequest,
  validatePromoteRequest,
} from "@42ch/spoke-operations";
import type {
  ComputeRequest,
  Finding,
  KnowledgeEntry,
  ProjectRequest,
  PromoteRequest,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { FIXTURES_ROOT } from "./schema-validator.js";

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

describe("fixtures/toy-world ops exercise", () => {
  const mira = () => loadFixture<KnowledgeEntry>("kb_tw_mira.json");
  const harbor = () => loadFixture<KnowledgeEntry>("kb_tw_harbor.json");
  const timelineEvent = () => loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json");
  const finding = () => loadFixture<Finding>("fnd_tw_open.json");

  it("passes promote gate for provisional Mira candidate", () => {
    const candidate: KnowledgeEntry = {
      ...mira(),
      status: "provisional",
    };
    const request: PromoteRequest = { candidate };

    expect(validatePromoteRequest(request).ok).toBe(true);
  });

  it("matches Scope toy-scope-001 refinements", () => {
    const scope: Scope = {
      scope_id: "toy-scope-001",
      entry_ids: ["kb_tw_mira", "kb_tw_harbor"],
      entry_types: ["character", "location"],
    };

    expect(knowledgeEntryMatchesScope(mira(), scope)).toBe(true);
    expect(knowledgeEntryMatchesScope(harbor(), scope)).toBe(true);
    expect(filterKnowledgeEntriesByScope([mira(), harbor()], scope)).toHaveLength(2);
    expect(
      knowledgeEntryMatchesScope(mira(), {
        scope_id: "toy-scope-001",
        source_id: "manuscript:tw-ch1",
      }),
    ).toBe(true);
    expect(
      timelineEventMatchesScope(timelineEvent(), {
        scope_id: "toy-scope-001",
        timeline_event_ids: ["evt_tw_harbor_dawn"],
        timeline_scale: "moment",
      }),
    ).toBe(true);
  });

  it("asserts revision match on fixture KnowledgeEntries", () => {
    expect(assertRevisionMatch(1, 1).ok).toBe(true);
    expect(assertRevisionMatch(1, 0).ok).toBe(false);
  });

  it("builds AssemblePacket from fixture KnowledgeEntries", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_tw_scope_built",
      knowledgeEntries: [mira(), harbor()],
    });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.entries).toHaveLength(2);
      expect(result.value.packet_id).toBe("pkt_tw_scope_built");
    }
  });

  it("transitions open finding to resolved", () => {
    const result = transitionFindingStatus(finding(), "resolved");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("resolved");
    }
  });

  it("validates optional project/compute op fixtures", () => {
    const projectRequest = loadFixture<ProjectRequest>("op_tw_project_request.json");
    const computeRequest = loadFixture<ComputeRequest>(
      "op_tw_compute_settle_request.json",
    );
    const timelineEvent = loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json");

    expect(validateProjectRequest(projectRequest).ok).toBe(true);
    expect(validateComputeRequest(computeRequest).ok).toBe(true);
    expect(
      validateComputableLogEntry(timelineEvent.computable_logs![0]!).ok,
    ).toBe(true);
  });
});
