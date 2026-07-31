import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import {
  filterKnowledgeEntriesByScope,
  filterTimelineEventsByScope,
  preserveExtensionMaps,
  timelineEventMatchesScope,
} from "@42ch/spoke-operations";
import type {
  AssemblePacket,
  ConnectHello,
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  FIXTURE_SCHEMA_MAP,
  FIXTURES_ROOT,
  compileSchemaValidator,
  createSchemaValidator,
} from "./schema-validator.js";

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

describe("fixtures/toy-world schema conformance", () => {
  const ajv = createSchemaValidator();

  const fixtureFiles = readdirSync(FIXTURES_ROOT).filter(
    (name) => name.endsWith(".json") && name !== "package.json",
  );

  it.each(fixtureFiles)("validates %s against its schema", (filename) => {
    const schemaId = FIXTURE_SCHEMA_MAP[filename];

    expect(schemaId, `missing schema mapping for ${filename}`).toBeDefined();

    const validate = compileSchemaValidator(ajv, schemaId!);
    const data = JSON.parse(
      readFileSync(join(FIXTURES_ROOT, filename), "utf8"),
    );

    const valid = validate(data);

    expect(validate.errors, JSON.stringify(validate.errors, null, 2)).toBeNull();
    expect(valid).toBe(true);
  });

  it("covers every mapped fixture file", () => {
    expect(fixtureFiles.sort()).toEqual(Object.keys(FIXTURE_SCHEMA_MAP).sort());
  });

  it("resolves graph ids within the toy-world graph", () => {
    const mira = loadFixture<KnowledgeEntry>("kb_tw_mira.json");
    const harbor = loadFixture<KnowledgeEntry>("kb_tw_harbor.json");
    const relation = loadFixture<{ from_id: string; to_id: string }>(
      "rel_tw_mira_harbor.json",
    );
    const timelineEvent = loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json");
    const finding = loadFixture<Finding>("fnd_tw_open.json");
    const packet = loadFixture<AssemblePacket>("pkt_tw_scope.json");

    expect(relation.from_id).toBe(mira.entry_id);
    expect(relation.to_id).toBe(harbor.entry_id);
    expect(timelineEvent.participant_entry_ids).toEqual(
      expect.arrayContaining([mira.entry_id, harbor.entry_id]),
    );
    expect(finding.target_entry_id).toBe(mira.entry_id);
    expect(packet.entries.map((entry) => entry.entry_id)).toEqual([
      mira.entry_id,
      harbor.entry_id,
    ]);
  });

  it("exercises dual-concern ontology event vs TimelineEvent (D5)", () => {
    const ontologyEvent = loadFixture<KnowledgeEntry>("kb_tw_harbor_dawn_event.json");
    const timelineEvent = loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json");

    expect(ontologyEvent.entry_type).toBe("event");
    expect(ontologyEvent.canonical_name).toBe(timelineEvent.canonical_name);
    expect(timelineEvent.extensions?.spoke).toEqual({
      timeline_entry_id: ontologyEvent.entry_id,
    });
  });

  it("preserves unknown extension keys on Mira KnowledgeEntry", () => {
    const mira = loadFixture<KnowledgeEntry>("kb_tw_mira.json");
    const preserved = preserveExtensionMaps(mira.extensions, {});

    expect(preserved.nexus).toEqual({
      world_id: "wld_toy_nexus_001",
      fork_hint: "baseline",
    });
    expect(preserved.creader).toEqual({
      book_id: "bk_toy_creader_001",
      chapter_slug: "harbor-arrival",
    });
  });

  it("illustrates BodyAttribute traits on Mira KnowledgeEntry", () => {
    const mira = loadFixture<KnowledgeEntry>("kb_tw_mira.json");

    expect(mira.body.attributes).toEqual([
      { trait_type: "role", value: "protagonist" },
      { trait_type: "age", value: 28, display_type: "number" },
    ]);
  });

  it("illustrates l2-computable body.state and body.computable on Harbor", () => {
    const harbor = loadFixture<KnowledgeEntry>("kb_tw_harbor.json");

    expect(harbor.body.state).toEqual({
      tide_level: 2.1,
      cargo_tons: 40,
    });
    expect(harbor.body.computable).toEqual({
      tide_level: 2.4,
      cargo_tons: 38,
    });
  });

  it("illustrates moment-scale computable_logs on harbor dawn TimelineEvent", () => {
    const harbor = loadFixture<KnowledgeEntry>("kb_tw_harbor.json");
    const timelineEvent = loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json");

    expect(timelineEvent.timeline_scale).toBe("moment");
    expect(timelineEvent.computable_logs).toEqual([
      {
        logged_at: "2026-07-23T05:50:00Z",
        entry_id: harbor.entry_id,
        changes: [
          { path: "tide_level", previous: 2.1, next: 2.4 },
          { path: "cargo_tons", previous: 40, next: 38 },
        ],
        session_id: "sess_tw_dawn_arrival",
        message: "Tide and cargo projection updated as Mira docks.",
      },
    ]);
  });

  it("keeps baseline harbor dawn TimelineEvent without Fork wire fields", () => {
    const baseline = loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json");

    expect(baseline.fork_id).toBeUndefined();
    expect(baseline.parent_fork_id).toBeUndefined();
  });

  it("illustrates l5-fork wire fields on storm-delay TimelineEvent", () => {
    const forked = loadFixture<TimelineEvent>("evt_tw_harbor_storm_delay.json");
    const mira = loadFixture<KnowledgeEntry>("kb_tw_mira.json");

    expect(forked.fork_id).toBe("fork_tw_storm_branch");
    expect(forked.parent_fork_id).toBe("fork_tw_mainline");
    expect(forked.participant_entry_ids).toEqual(
      expect.arrayContaining([mira.entry_id, "kb_tw_harbor"]),
    );
    expect(forked.extensions?.nexus).toEqual({
      world_id: "wld_toy_nexus_001",
    });
    expect(mira.extensions?.nexus?.fork_hint).toBe("baseline");
  });

  it("illustrates disjoint host manifest namespaces across toy-world hosts", () => {
    const primary = loadFixture<HostCapabilityManifest>("host_tw_primary.json");
    const peer = loadFixture<HostCapabilityManifest>("host_tw_peer.json");

    expect(primary.host_id).not.toBe(peer.host_id);
    expect(primary.roles).toContain("assembler");
    expect(primary.roles).toContain("data-store");
    expect(primary.capabilities).toContain("spoke-baseline");
    expect(primary.roles).not.toContain("computable-engine");
    expect(peer.roles).not.toContain("computable-engine");
    expect(peer.capabilities).toContain("spoke-baseline");

    const primaryNamespaces = new Set(primary.namespaces);
    const peerNamespaces = new Set(peer.namespaces);
    const overlap = [...primaryNamespaces].filter((ns) => peerNamespaces.has(ns));

    expect(overlap).toEqual([]);
  });

  it("filters TimelineEvents by Scope.fork_id via operations helper", () => {
    const baseline = loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json");
    const forked = loadFixture<TimelineEvent>("evt_tw_harbor_storm_delay.json");
    const events = [baseline, forked];

    expect(
      timelineEventMatchesScope(forked, {
        scope_id: "toy-scope-fork",
        fork_id: "fork_tw_storm_branch",
      }),
    ).toBe(true);
    expect(
      timelineEventMatchesScope(baseline, {
        scope_id: "toy-scope-fork",
        fork_id: "fork_tw_storm_branch",
      }),
    ).toBe(false);
    expect(
      filterTimelineEventsByScope(events, {
        scope_id: "toy-scope-fork",
        fork_id: "fork_tw_storm_branch",
      }),
    ).toEqual([forked]);
    expect(
      filterTimelineEventsByScope(events, {
        scope_id: "toy-scope-baseline",
      }),
    ).toEqual(events);
  });

  it("illustrates connect dual identity and structural hello signatures", () => {
    const primary = loadFixture<ConnectHello>("conn_tw_hello_primary_to_peer.json");
    const peer = loadFixture<ConnectHello>("conn_tw_hello_peer_to_primary.json");

    // peer_id (network identity, trust root) is distinct from the embedded
    // host_id (application identity) — the protocol does not require equality.
    expect(primary.peer_id).toBe("peer_tw_primary");
    expect(primary.host.host_id).toBe("host_tw_primary");
    expect(primary.peer_id).not.toBe(primary.host.host_id);
    expect(peer.peer_id).toBe("peer_tw_peer");
    expect(peer.host.host_id).toBe("host_tw_peer");

    // Nonces are single-use replay guards (>= 16 chars on the wire).
    expect(primary.nonce.length).toBeGreaterThanOrEqual(16);
    expect(peer.nonce).not.toBe(primary.nonce);

    // Structural conformance only: signatures are valid-shaped base64url test
    // vectors (no padding), not cryptographically real. JCS signing + verify is
    // the responsibility of the reference stack (spoke-connect spike).
    expect(primary.signature).toMatch(/^[A-Za-z0-9_-]+$/);
    expect(primary.signature.length).toBe(86); // 64 raw bytes, no padding
    expect(peer.signature).toMatch(/^[A-Za-z0-9_-]+$/);
    expect(peer.signature).not.toBe(primary.signature);

    // Embedded manifests are adapted from the committed baseline hosts with the
    // optional spoke-connect capability added; the standalone manifests stay
    // baseline (spoke-connect is opt-in).
    expect(primary.host.capabilities).toEqual(["spoke-baseline", "spoke-connect"]);
    expect(peer.host.capabilities).toEqual(["spoke-baseline", "spoke-connect"]);
  });

  it("echoes session and request correlation on connect invoke responses", () => {
    const request = loadFixture<{ session_id: string; sequence: number; request_id: string }>(
      "conn_tw_invoke_check_request.json",
    );
    const success = loadFixture<{ session_id: string; sequence: number; request_id: string }>(
      "conn_tw_invoke_check_response.json",
    );
    const error = loadFixture<{ session_id: string; sequence: number; request_id: string }>(
      "conn_tw_invoke_error_response.json",
    );

    expect(success.session_id).toBe(request.session_id);
    expect(success.sequence).toBe(request.sequence);
    expect(success.request_id).toBe(request.request_id);

    // Error branch echoes the failing invoke (a second, malformed request).
    expect(error.session_id).toBe(request.session_id);
    expect(error.sequence).toBe(1);
    expect(error.request_id).toBe("req_tw_check_0002");
  });

  it("carries product query metadata on Scope.extensions through a check-request", () => {
    const checkRequest = loadFixture<{ scope: Scope }>("op_tw_check_request.json");
    const scope = checkRequest.scope;

    // The selector carries product-scoped query metadata under extensions.<namespace>.
    expect(scope.extensions).toEqual({
      toy: { branch_id: "fork_tw_storm_branch" },
      nexus: { text_search: "harbor", limit: 10 },
    });

    // Matching still selects on core fields (entry_types) using the real toy-world corpus.
    const mira = loadFixture<KnowledgeEntry>("kb_tw_mira.json");
    const harbor = loadFixture<KnowledgeEntry>("kb_tw_harbor.json");

    const filtered = filterKnowledgeEntriesByScope([mira, harbor], scope);
    expect(filtered.map((entry) => entry.entry_id)).toEqual([mira.entry_id]);

    // Same core selection as an extensions-less scope.
    const coreOnly: Scope = {
      scope_id: scope.scope_id,
      entry_types: scope.entry_types,
    };
    expect(
      filterKnowledgeEntriesByScope([mira, harbor], coreOnly).map((entry) => entry.entry_id),
    ).toEqual([mira.entry_id]);

    // extensions are preserved verbatim — matchers neither consume nor alter them.
    const snapshot = scope.extensions;
    const baseline = loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json");
    const forked = loadFixture<TimelineEvent>("evt_tw_harbor_storm_delay.json");
    filterTimelineEventsByScope([baseline, forked], scope);

    expect(scope.extensions).toBe(snapshot);
    expect(scope.extensions).toEqual({
      toy: { branch_id: "fork_tw_storm_branch" },
      nexus: { text_search: "harbor", limit: 10 },
    });
  });
});
