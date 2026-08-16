/**
 * Deterministic toy-world tool handlers — the copyable provider reference.
 *
 * Both handlers are pure functions of their arguments (no I/O, no global
 * state): `roll_dice` derives a seed from the arguments, `lore_lookup`
 * reads the adapter store without mutating it. Same arguments always
 * produce the same result, so e2e assertions are stable.
 */

import {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

import type { MemoryStore } from "./memory-store.js";

/** Frozen tool capability ids (docs/snippet byte-parity). */
export const TOY_WORLD_ROLL_DICE_ID = "tools.toy_world.roll_dice";
export const TOY_WORLD_LORE_LOOKUP_ID = "tools.toy_world.lore_lookup";

/**
 * Tool handler shape — mirrors the plan-3 serving surfaces
 * (`RemoteAdapter.registerToolHandler` / `connectResponder`), kept local so
 * the fixture stays dependency-free beyond operations/schemas.
 */
export type ToolHandler = (
  args: Record<string, unknown>,
) => Promise<SpokeResult<unknown>>;

/** 32-bit FNV-1a hash — stable across runs and platforms. */
function fnv1a(input: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/**
 * mulberry32 PRNG — deterministic for a given 32-bit seed.
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
 * Deterministic dice handler: `count` rolls of `sides`-sided dice.
 * The seed is derived from `count` and `sides`, so the same arguments
 * always produce the same rolls. `sides` defaults to 6.
 */
export function rollDice(
  args: Record<string, unknown>,
): Promise<SpokeResult<unknown>> {
  const count = args["count"];
  const sides = args["sides"] ?? 6;
  if (!isPositiveInteger(count)) {
    return Promise.resolve(
      spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        "roll_dice count must be a positive integer",
        { field: "count" },
      ),
    );
  }
  if (!isPositiveInteger(sides) || sides < 2) {
    return Promise.resolve(
      spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        "roll_dice sides must be an integer >= 2",
        { field: "sides" },
      ),
    );
  }

  const random = mulberry32(fnv1a(`${count}:${sides}`));
  const rolls: number[] = [];
  for (let index = 0; index < count; index += 1) {
    rolls.push(1 + Math.floor(random() * sides));
  }
  const total = rolls.reduce((sum, roll) => sum + roll, 0);
  return Promise.resolve(spokeOk({ rolls, total }));
}

/**
 * Read-only lore handler: look up a toy-world KnowledgeEntry by id from the
 * adapter store. The store's own reject (e.g. KNOWLEDGE_ENTRY_NOT_FOUND)
 * passes through unchanged.
 */
export function loreLookup(store: MemoryStore): ToolHandler {
  return async (args) => {
    const entryId = args["entry_id"];
    if (typeof entryId !== "string" || entryId.length === 0) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        "lore_lookup entry_id must be a non-empty string",
        { field: "entry_id" },
      );
    }
    const result = store.getKnowledgeEntry(entryId);
    if (!result.ok) {
      return result;
    }
    return spokeOk({ entry: result.value });
  };
}

/**
 * Default handler registry for a toy-world adapter: both frozen tools are
 * servable out of the box. `lore_lookup` is bound to the adapter store.
 */
export function defaultToyWorldToolHandlers(
  store: MemoryStore,
): Map<string, ToolHandler> {
  return new Map([
    [TOY_WORLD_ROLL_DICE_ID, rollDice],
    [TOY_WORLD_LORE_LOOKUP_ID, loreLookup(store)],
  ]);
}
