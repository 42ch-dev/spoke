import { describe, expect, it } from "vitest";

import type {
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
  ProjectRequest,
  ProjectResponse,
  ComputeRequest,
  ComputeResponse,
  ForkId,
} from "@42ch/spoke-schemas";

import {
  SpokeRejectCode,
  spokeOk,
  type SpokeResult,
  type KnowledgeEntryPort,
  type RelationPort,
  type ScopeQueryPort,
  type FindingPort,
  type RuleQueryPort,
  type HostManifestPort,
  type ComputablePort,
  type ForkTimelineQueryPort,
  type BaselinePorts,
  type ComputablePorts,
  type ForkPorts,
  type FullPorts,
  type BaselineAdapter,
  type ComputableAdapter,
  type ForkAdapter,
  type FullAdapter,
} from "../index.js";

function makeManifest(
  overrides: Partial<HostCapabilityManifest> & Pick<HostCapabilityManifest, "host_id">,
): HostCapabilityManifest {
  return {
    schema_version: 1,
    roles: ["data-store"],
    capabilities: ["spoke-baseline"],
    namespaces: ["default"],
    extensions: {},
    ...overrides,
  };
}

function normalizePeerManifests(
  selfHostId: string,
  peers: HostCapabilityManifest[],
): HostCapabilityManifest[] {
  const byHostId = new Map<string, HostCapabilityManifest>();
  for (const peer of peers) {
    if (peer.host_id === selfHostId) {
      continue;
    }
    byHostId.set(peer.host_id, peer);
  }
  return [...byHostId.values()].sort((left, right) =>
    Buffer.compare(
      Buffer.from(left.host_id, "utf8"),
      Buffer.from(right.host_id, "utf8"),
    ),
  );
}

function createHostManifestPortMock(seed?: {
  self?: HostCapabilityManifest;
  peers?: HostCapabilityManifest[];
}): HostManifestPort {
  const self =
    seed?.self ??
    makeManifest({
      host_id: "self-host",
      namespaces: ["self-ns"],
    });
  const rawPeers = seed?.peers ?? [];

  return {
    getHostCapabilityManifest(): SpokeResult<HostCapabilityManifest> {
      return spokeOk(self);
    },
    listPeerHostCapabilityManifests(): SpokeResult<HostCapabilityManifest[]> {
      return spokeOk(normalizePeerManifests(self.host_id, rawPeers));
    },
  };
}

function createBaselinePortStub(
  hostManifest?: HostManifestPort,
): BaselinePorts {
  return {
    getKnowledgeEntry: () => spokeOk({} as KnowledgeEntry),
    putKnowledgeEntry: (entry, _expectedBaseRevision) => spokeOk(entry),
    getRelation: () => spokeOk({} as Relation),
    putRelation: (relation, _expectedBaseRevision) => spokeOk(relation),
    listKnowledgeEntries: () => spokeOk([]),
    listTimelineEvents: () => spokeOk([]),
    putFindings: (findings) => spokeOk(findings),
    listRules: () => spokeOk([]),
    ...createHostManifestPortMock(),
    ...hostManifest,
  };
}

describe("adapter port exports", () => {
  it("exports CAPABILITY_PORT_MISSING as the 22nd SpokeRejectCode", () => {
    expect(SpokeRejectCode.CAPABILITY_PORT_MISSING).toBe(
      "CAPABILITY_PORT_MISSING",
    );
    expect(Object.keys(SpokeRejectCode)).toHaveLength(22);
  });

  it("BaselinePorts accepts an object implementing all six baseline families", () => {
    const ports: BaselinePorts = createBaselinePortStub();

    expect(typeof ports.getKnowledgeEntry).toBe("function");
    expect(typeof ports.getRelation).toBe("function");
    expect(typeof ports.putRelation).toBe("function");
    expect(typeof ports.listKnowledgeEntries).toBe("function");
    expect(typeof ports.putFindings).toBe("function");
    expect(typeof ports.listRules).toBe("function");
    expect(typeof ports.getHostCapabilityManifest).toBe("function");
    expect(typeof ports.listPeerHostCapabilityManifests).toBe("function");
  });

  it("RelationPort shape exposes getRelation + putRelation(relation, expectedBaseRevision)", () => {
    const relation: RelationPort = {
      getRelation: () => spokeOk({} as Relation),
      putRelation: (r, _expectedBaseRevision) => spokeOk(r),
    };

    // The port requires a two-arg putRelation (OCC expectedBaseRevision) and a
    // getRelation reader — parity with KnowledgeEntryPort and the Rust trait.
    expect(typeof relation.getRelation).toBe("function");
    expect(typeof relation.putRelation).toBe("function");
    expect(relation.putRelation.length).toBe(2);
  });

  it("ComputablePorts / ForkPorts / FullPorts compose optional families", () => {
    const baseline: BaselinePorts = createBaselinePortStub();

    const computable: ComputablePort = {
      project(request: ProjectRequest): SpokeResult<ProjectResponse> {
        return spokeOk({} as ProjectResponse);
      },
      compute(request: ComputeRequest): SpokeResult<ComputeResponse> {
        return spokeOk({} as ComputeResponse);
      },
    };

    const fork: ForkTimelineQueryPort = {
      listForkTimelineEvents(
        _scope: Scope & { fork_id: ForkId },
      ): SpokeResult<TimelineEvent[]> {
        return spokeOk([]);
      },
    };

    const computablePorts: ComputablePorts = { ...baseline, ...computable };
    const forkPorts: ForkPorts = { ...baseline, ...fork };
    const fullPorts: FullPorts = { ...baseline, ...computable, ...fork };

    expect(typeof computablePorts.project).toBe("function");
    expect(typeof forkPorts.listForkTimelineEvents).toBe("function");
    expect(typeof fullPorts.compute).toBe("function");
    expect(typeof fullPorts.listForkTimelineEvents).toBe("function");
  });

  it("*Adapter aliases are assignable from matching *Ports compositions", () => {
    const baseline: BaselinePorts = createBaselinePortStub();

    const computable: ComputablePort = {
      project: () => spokeOk({} as ProjectResponse),
      compute: () => spokeOk({} as ComputeResponse),
    };

    const fork: ForkTimelineQueryPort = {
      listForkTimelineEvents: () => spokeOk([]),
    };

    const baselineAdapter: BaselineAdapter = baseline;
    const computableAdapter: ComputableAdapter = { ...baseline, ...computable };
    const forkAdapter: ForkAdapter = { ...baseline, ...fork };
    const fullAdapter: FullAdapter = { ...baseline, ...computable, ...fork };

    expect(typeof baselineAdapter.getKnowledgeEntry).toBe("function");
    expect(typeof computableAdapter.project).toBe("function");
    expect(typeof forkAdapter.listForkTimelineEvents).toBe("function");
    expect(typeof fullAdapter.compute).toBe("function");
  });

  it("individual port interfaces are assignable by method shape", () => {
    const knowledge: KnowledgeEntryPort = {
      getKnowledgeEntry: () => spokeOk({} as KnowledgeEntry),
      putKnowledgeEntry: (entry, _expectedBaseRevision) => spokeOk(entry),
    };
    const relation: RelationPort = {
      getRelation: () => spokeOk({} as Relation),
      putRelation: (r, _expectedBaseRevision) => spokeOk(r),
    };
    const scope: ScopeQueryPort = {
      listKnowledgeEntries: () => spokeOk([]),
      listTimelineEvents: () => spokeOk([]),
    };
    const finding: FindingPort = {
      putFindings: (f) => spokeOk(f),
    };
    const rule: RuleQueryPort = {
      listRules: () => spokeOk([]),
    };
    const hostManifest: HostManifestPort = createHostManifestPortMock();

    expect(knowledge).toBeDefined();
    expect(relation).toBeDefined();
    expect(scope).toBeDefined();
    expect(finding).toBeDefined();
    expect(rule).toBeDefined();
    expect(hostManifest).toBeDefined();
  });
});

describe("HostManifestPort", () => {
  it("returns the self manifest from getHostCapabilityManifest", () => {
    const self = makeManifest({
      host_id: "adapter-self",
      namespaces: ["alpha"],
      roles: ["data-store", "checker"],
    });
    const ports = createHostManifestPortMock({ self });

    const result = ports.getHostCapabilityManifest();

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual(self);
  });

  it("accepts an empty peer list", () => {
    const ports = createHostManifestPortMock({ peers: [] });

    const result = ports.listPeerHostCapabilityManifests();

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual([]);
  });

  it("returns seeded peer manifests with disjoint namespaces", () => {
    const self = makeManifest({ host_id: "self-host", namespaces: ["self-ns"] });
    const peerA = makeManifest({
      host_id: "peer-a",
      namespaces: ["peer-a-ns"],
      roles: ["checker"],
    });
    const peerB = makeManifest({
      host_id: "peer-b",
      namespaces: ["peer-b-ns"],
      roles: ["assembler"],
    });
    const ports = createHostManifestPortMock({
      self,
      peers: [peerB, peerA],
    });

    const result = ports.listPeerHostCapabilityManifests();

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual([peerA, peerB]);
    const namespaces = result.value.flatMap((manifest) => manifest.namespaces);
    expect(new Set(namespaces).size).toBe(namespaces.length);
    expect(namespaces).not.toContain("self-ns");
  });

  it("excludes self, dedupes by host_id, and sorts peers ascending by host_id", () => {
    const self = makeManifest({ host_id: "self-host", namespaces: ["self-ns"] });
    const peerZ = makeManifest({ host_id: "peer-z", namespaces: ["z-ns"] });
    const peerADupe = makeManifest({
      host_id: "peer-a",
      namespaces: ["a-ns-dup"],
      roles: ["checker"],
    });
    const peerA = makeManifest({
      host_id: "peer-a",
      namespaces: ["a-ns"],
      roles: ["assembler"],
    });
    const ports = createHostManifestPortMock({
      self,
      peers: [peerZ, self, peerADupe, peerA],
    });

    const result = ports.listPeerHostCapabilityManifests();

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.map((manifest) => manifest.host_id)).toEqual([
      "peer-a",
      "peer-z",
    ]);
    expect(result.value[0]).toEqual(peerA);
  });
});
