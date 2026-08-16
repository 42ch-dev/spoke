/**
 * Deterministic toy-world tool handlers for the demo client — the
 * copyable-provider pattern.
 *
 * These are the SAME frozen tool ids + deterministic algorithms as the
 * reference provider (`fixtures/toy-world/src/adapter/toy-world-tools.ts`):
 * `roll_dice` derives a seed from its arguments (FNV-1a + mulberry32), so
 * the same arguments always produce the same rolls; `lore_lookup` reads the
 * client's own in-memory lore store without mutating it.
 *
 * The demo client must NOT import the private fixture package (runtime-dep
 * lean: only `@42ch/spoke-connect` + `@42ch/spoke-schemas` + `ws`), so the
 * handlers live here, byte-compatible with the fixture. `validateManifestTools`
 * (spoke-operations, server-side) guarantees the client manifest's tools[]
 * matches these ids.
 */

import type { KnowledgeEntry, ToolDescriptor } from "@42ch/spoke-schemas";

/** Frozen tool capability ids (docs/snippet byte-parity with the fixture). */
export const TOY_WORLD_ROLL_DICE_ID = "tools.toy_world.roll_dice";
export const TOY_WORLD_LORE_LOOKUP_ID = "tools.toy_world.lore_lookup";

/**
 * Tool-handler result — structural subset of `SpokeResult` (the demo client
 * does not import `@42ch/spoke-operations`). The reject codes are SpokeReject
 * code literals, so a handler of this shape type-checks as the library's
 * `ToolHandler` when registered on a RemoteAdapter.
 */
export type ToolRejectCode =
  | "INVALID_INPUT"
  | "KNOWLEDGE_ENTRY_NOT_FOUND"
  | "CAPABILITY_PORT_MISSING"
  | "INTERNAL_ERROR";

export type ToolResult =
  | { ok: true; value: unknown }
  | {
      ok: false;
      code: ToolRejectCode;
      message: string;
      details?: Record<string, unknown>;
    };

/** Tool-handler shape — mirrors the library `ToolHandler` (positional `args`). */
export type ToolHandler = (args: Record<string, unknown>) => Promise<ToolResult>;

function reject(
  code: ToolRejectCode,
  message: string,
  details?: Record<string, unknown>,
): ToolResult {
  return { ok: false, code, message, ...(details !== undefined ? { details } : {}) };
}

/** 32-bit FNV-1a hash — stable across runs and platforms (fixture parity). */
function fnv1a(input: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/**
 * mulberry32 PRNG — deterministic for a given 32-bit seed (fixture parity).
 * Returns values in [0, 1).
 */
function mulberry32(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 1;
}

/**
 * Deterministic dice handler: `count` rolls of `sides`-sided dice. The seed
 * is derived from `count` and `sides`, so the same arguments always produce
 * the same rolls. `sides` defaults to 6.
 */
export function rollDice(args: Record<string, unknown>): Promise<ToolResult> {
  const count = args["count"];
  const sides = args["sides"] ?? 6;
  if (!isPositiveInteger(count)) {
    return Promise.resolve(
      reject("INVALID_INPUT", "roll_dice count must be a positive integer", {
        field: "count",
      }),
    );
  }
  if (!isPositiveInteger(sides) || sides < 2) {
    return Promise.resolve(
      reject("INVALID_INPUT", "roll_dice sides must be an integer >= 2", {
        field: "sides",
      }),
    );
  }

  const random = mulberry32(fnv1a(`${count}:${sides}`));
  const rolls: number[] = [];
  for (let index = 0; index < count; index += 1) {
    rolls.push(1 + Math.floor(random() * sides));
  }
  const total = rolls.reduce((sum, roll) => sum + roll, 0);
  return Promise.resolve({ ok: true, value: { rolls, total } });
}

/**
 * Read-only lore handler: look up a KnowledgeEntry the client knows about
 * (its own submitted entries, kept in the in-memory lore store). The store's
 * reject codes pass through unchanged.
 */
export function loreLookup(store: Map<string, KnowledgeEntry>): ToolHandler {
  return async (args) => {
    const entryId = args["entry_id"];
    if (typeof entryId !== "string" || entryId.length === 0) {
      return reject(
        "INVALID_INPUT",
        "lore_lookup entry_id must be a non-empty string",
        { field: "entry_id" },
      );
    }
    const entry = store.get(entryId);
    if (entry === undefined) {
      return reject(
        "KNOWLEDGE_ENTRY_NOT_FOUND",
        `KnowledgeEntry not found: ${entryId}`,
        { entry_id: entryId },
      );
    }
    return { ok: true, value: { entry } };
  };
}

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
