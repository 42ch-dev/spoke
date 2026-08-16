/**
 * Toy-world tool constants for the demo server — the frozen ids and
 * descriptors the demo negotiates.
 *
 * The demo is the product-visible proof of the reverse-tool surface: the
 * client exposes `tools.toy_world.roll_dice` and `tools.toy_world.lore_lookup`
 * (ids frozen for docs/snippet byte-parity with the reference provider
 * `fixtures/toy-world/host_tw_primary.json` and the demo client), and the
 * host lists them from the authenticated manifest and reverse-invokes them
 * mid-orchestration. Both sides must list the ids in `capabilities[]` for
 * them to be negotiated.
 */

import type { ToolDescriptor } from "@42ch/spoke-schemas";

/** Frozen tool capability ids (docs/snippet byte-parity). */
export const TOY_WORLD_ROLL_DICE_ID = "tools.toy_world.roll_dice";
export const TOY_WORLD_LORE_LOOKUP_ID = "tools.toy_world.lore_lookup";

/** The tool namespace both demo manifests own (`namespaces[]`). */
export const TOY_WORLD_NAMESPACE = "toy_world";

/** roll_dice ToolDescriptor (frozen — byte-parity with the fixture manifest). */
export const ROLL_DICE_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: TOY_WORLD_ROLL_DICE_ID,
  op: TOY_WORLD_ROLL_DICE_ID,
  description:
    "Roll `count` dice with `sides` faces each. Deterministic: the same arguments always produce the same rolls (seeded from the arguments).",
  input: {
    type: "object",
    properties: {
      count: { type: "integer", minimum: 1 },
      sides: { type: "integer", minimum: 2 },
    },
    required: ["count"],
  },
  output: {
    type: "object",
    properties: {
      rolls: {
        type: "array",
        items: { type: "integer" },
      },
      total: { type: "integer" },
    },
    required: ["rolls", "total"],
  },
  idempotent: true,
};

/** lore_lookup ToolDescriptor (frozen — byte-parity with the fixture manifest). */
export const LORE_LOOKUP_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: TOY_WORLD_LORE_LOOKUP_ID,
  op: TOY_WORLD_LORE_LOOKUP_ID,
  description:
    "Look up a knowledge entry by id from the client's own lore store. Read-only and deterministic.",
  input: {
    type: "object",
    properties: {
      entry_id: { type: "string" },
    },
    required: ["entry_id"],
  },
  output: {
    type: "object",
    properties: {
      entry: { type: "object" },
    },
    required: ["entry"],
  },
  idempotent: true,
};
