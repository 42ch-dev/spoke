/**
 * Demo seed corpus for the mock inference engine — the fixed starting store
 * history of the connect demo. All ids/revisions are literal constants
 * (determinism constraint: no wall-clock or random values anywhere in the
 * engine's asserted outputs).
 */

import type { KnowledgeEntry, Relation, Rule } from "@42ch/spoke-schemas";

/** The single demo scope: every seed entity and every demo manifest belongs here. */
export const DEMO_SCOPE_ID = "demo-harbor";

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
