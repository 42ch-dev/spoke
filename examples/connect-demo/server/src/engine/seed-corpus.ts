/**
 * Demo seed corpus for the mock inference engine — the fixed starting store
 * history of the connect demo. All ids/revisions are literal constants
 * (determinism constraint: no wall-clock or random values anywhere in the
 * engine's asserted outputs).
 */

import type {
  KnowledgeEntry,
  Relation,
  Rule,
  TimelineEvent,
} from "@42ch/spoke-schemas";

/** The single demo scope: every seed entity and every demo manifest belongs here. */
export const DEMO_SCOPE_ID = "demo-harbor";

/** The seeded l5-fork branch: a storm timeline forking the harbor world. */
export const DEMO_SEED_FORK_ID = "demo-harbor/fork/storm";

/** Seed KnowledgeEntries — 2 entries in scope `demo-harbor`. */
export const DEMO_SEED_ENTRIES: KnowledgeEntry[] = [
  {
    schema_version: 1,
    entry_id: "demo-harbor/character/mira",
    entry_type: "character",
    canonical_name: "Mira",
    status: "confirmed",
    body: { summary: "A dockworker who keeps the harbor log." },
    revision: 1,
    extensions: {},
  },
  {
    schema_version: 1,
    entry_id: "demo-harbor/location/harbor",
    entry_type: "location",
    canonical_name: "Harbor",
    status: "confirmed",
    body: { summary: "The demo harbor district." },
    revision: 1,
    extensions: {},
  },
];

/** Seed Relations — 1 relation connecting both seed entries. */
export const DEMO_SEED_RELATIONS: Relation[] = [
  {
    schema_version: 1,
    relation_id: "demo-harbor/relation/mira-located-in-harbor",
    relation_type: "located_in",
    from_id: "demo-harbor/character/mira",
    to_id: "demo-harbor/location/harbor",
    revision: 1,
    extensions: {},
  },
];

/**
 * Seed TimelineEvents — 3 events on the storm fork (`demo-harbor/fork/storm`),
 * the demo's l5-fork story. Deterministic literals: ids, labels, and
 * ordering hints are frozen so `listForkTimelineEvents` is a stable
 * function of the seed corpus.
 */
export const DEMO_SEED_TIMELINE_EVENTS: TimelineEvent[] = [
  {
    schema_version: 1,
    timeline_event_id: "demo-harbor/event/storm-landfall",
    canonical_name: "Storm makes landfall",
    occurred_at: "day 3",
    description: "The storm breaks over the harbor; waves top the sea wall.",
    participant_entry_ids: [
      "demo-harbor/character/mira",
      "demo-harbor/location/harbor",
    ],
    sort_key: "storm-001",
    fork_id: DEMO_SEED_FORK_ID,
    extensions: {},
  },
  {
    schema_version: 1,
    timeline_event_id: "demo-harbor/event/harbor-evacuation",
    canonical_name: "Harbor master orders evacuation",
    occurred_at: "day 3",
    description: "The harbor master closes the docks and moves the crew inland.",
    participant_entry_ids: [
      "demo-harbor/character/mira",
      "demo-harbor/location/harbor",
    ],
    sort_key: "storm-002",
    fork_id: DEMO_SEED_FORK_ID,
    extensions: {},
  },
  {
    schema_version: 1,
    timeline_event_id: "demo-harbor/event/compass-secured",
    canonical_name: "Mira secures the compass",
    occurred_at: "day 4",
    description: "Mira lashes the compass to her pack before the sea wall fails.",
    participant_entry_ids: ["demo-harbor/character/mira"],
    sort_key: "storm-003",
    fork_id: DEMO_SEED_FORK_ID,
    extensions: {},
  },
];

/** Seed Rules — 1 declarative rule backing the engine's `isolated_entry` findings. */
export const DEMO_SEED_RULES: Rule[] = [
  {
    schema_version: 1,
    rule_id: "demo-harbor/rule/no-isolated-entries",
    canonical_name: "No isolated entries",
    kind: "rule",
    statement:
      "Every knowledge entry must participate in at least one relation.",
    severity_hint: "warning",
    status: "active",
    extensions: {},
  },
];
