import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  assertRevisionMatch,
  buildAssemblePacket,
  eventMatchesScope,
  filterKeyblocksByScope,
  keyblockMatchesScope,
  transitionFindingStatus,
  validatePromoteRequest,
} from "@42ch/spoke-operations";
import type {
  Event,
  Finding,
  Keyblock,
  PromoteRequest,
  Scope,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { FIXTURES_ROOT } from "./schema-validator.js";

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

describe("fixtures/toy-world ops exercise", () => {
  const mira = () => loadFixture<Keyblock>("kb_tw_mira.json");
  const harbor = () => loadFixture<Keyblock>("kb_tw_harbor.json");
  const event = () => loadFixture<Event>("evt_tw_harbor_dawn.json");
  const finding = () => loadFixture<Finding>("fnd_tw_open.json");

  it("passes promote gate for provisional Mira candidate", () => {
    const candidate: Keyblock = {
      ...mira(),
      status: "provisional",
    };
    const request: PromoteRequest = { candidate };

    expect(validatePromoteRequest(request).ok).toBe(true);
  });

  it("matches Scope toy-scope-001 refinements", () => {
    const scope: Scope = {
      scope_id: "toy-scope-001",
      keyblock_ids: ["kb_tw_mira", "kb_tw_harbor"],
      block_types: ["character", "location"],
    };

    expect(keyblockMatchesScope(mira(), scope)).toBe(true);
    expect(keyblockMatchesScope(harbor(), scope)).toBe(true);
    expect(filterKeyblocksByScope([mira(), harbor()], scope)).toHaveLength(2);
    expect(
      keyblockMatchesScope(mira(), {
        scope_id: "toy-scope-001",
        source_id: "manuscript:tw-ch1",
      }),
    ).toBe(true);
    expect(
      eventMatchesScope(event(), {
        scope_id: "toy-scope-001",
        event_ids: ["evt_tw_harbor_dawn"],
        timeline_scale: "moment",
      }),
    ).toBe(true);
  });

  it("asserts revision match on fixture Keyblocks", () => {
    expect(assertRevisionMatch(1, 1).ok).toBe(true);
    expect(assertRevisionMatch(1, 0).ok).toBe(false);
  });

  it("builds AssemblePacket from fixture Keyblocks", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_tw_scope_built",
      keyblocks: [mira(), harbor()],
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
});
