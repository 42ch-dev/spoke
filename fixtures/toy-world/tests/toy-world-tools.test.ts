/**
 * Tool-surface parity tests for the toy-world reference provider:
 * manifest consistency (plan-2 validateManifestTools) + deterministic
 * handlers (plan-3 serving surface).
 */

import {
  SpokeRejectCode,
  findTool,
  listTools,
  validateManifestTools,
} from "@42ch/spoke-operations";
import type { HostCapabilityManifest, ToolDescriptor } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  TOY_WORLD_LORE_LOOKUP_ID,
  TOY_WORLD_ROLL_DICE_ID,
  ToyWorldAdapter,
} from "../src/adapter/index.js";

/** Frozen tool ids (docs/snippet byte-parity). */
const TOY_WORLD_TOOL_IDS = [
  TOY_WORLD_ROLL_DICE_ID,
  TOY_WORLD_LORE_LOOKUP_ID,
] as const;

/** Narrow a roll_dice success value: `{ rolls: number[]; total: number }`. */
function asRollResult(value: unknown): { rolls: number[]; total: number } {
  if (
    typeof value === "object" &&
    value !== null &&
    "rolls" in value &&
    Array.isArray(value.rolls) &&
    "total" in value &&
    typeof value.total === "number"
  ) {
    return { rolls: value.rolls, total: value.total };
  }
  throw new Error("roll_dice success value does not match { rolls, total }");
}

describe("toy-world tool provider reference", () => {
  it("declares both frozen tools in the adapter manifest and passes validateManifestTools", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const manifestResult = await adapter.getHostCapabilityManifest();
    expect(manifestResult.ok).toBe(true);
    if (!manifestResult.ok) {
      return;
    }
    const manifest: HostCapabilityManifest = manifestResult.value;

    const validated = validateManifestTools(manifest);
    expect(validated.ok, validated.ok ? undefined : validated.message).toBe(
      true,
    );

    const descriptors = listTools(manifest);
    expect(descriptors.map((descriptor) => descriptor.capability_id)).toEqual([
      ...TOY_WORLD_TOOL_IDS,
    ]);

    for (const capabilityId of TOY_WORLD_TOOL_IDS) {
      expect(manifest.capabilities).toContain(capabilityId);
      const namespace = capabilityId.split(".")[1];
      expect(manifest.namespaces).toContain(namespace);
      const descriptor = findTool(manifest, capabilityId);
      expect(descriptor?.op).toBe(capabilityId);
      expect(descriptor?.input).toBeTypeOf("object");
      expect(descriptor?.output).toBeTypeOf("object");
    }
  });

  it("toolDescriptors() lists the manifest tools as a defensive copy", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const descriptors = adapter.toolDescriptors();
    expect(descriptors.map((descriptor) => descriptor.capability_id)).toEqual([
      ...TOY_WORLD_TOOL_IDS,
    ]);

    // Mutating the returned array must not mutate the adapter's manifest.
    descriptors.push({} as ToolDescriptor);
    expect(adapter.toolDescriptors()).toHaveLength(TOY_WORLD_TOOL_IDS.length);
  });

  it("roll_dice is deterministic: same arguments produce identical rolls", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const args = { count: 3, sides: 6 };

    const first = await adapter.invokeTool(TOY_WORLD_ROLL_DICE_ID, args);
    const second = await adapter.invokeTool(TOY_WORLD_ROLL_DICE_ID, args);

    expect(first).toEqual(second);
    expect(first.ok).toBe(true);
    if (!first.ok) {
      return;
    }
    const { rolls, total } = asRollResult(first.value);
    expect(rolls).toHaveLength(3);
    for (const roll of rolls) {
      expect(roll).toBeGreaterThanOrEqual(1);
      expect(roll).toBeLessThanOrEqual(6);
    }
    expect(total).toBe(rolls.reduce((sum, roll) => sum + roll, 0));
  });

  it("roll_dice defaults sides to 6 and rejects invalid count", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();

    const defaulted = await adapter.invokeTool(TOY_WORLD_ROLL_DICE_ID, {
      count: 1,
    });
    expect(defaulted.ok).toBe(true);
    if (defaulted.ok) {
      const { rolls } = asRollResult(defaulted.value);
      expect(rolls).toHaveLength(1);
      expect(rolls[0]).toBeGreaterThanOrEqual(1);
      expect(rolls[0]).toBeLessThanOrEqual(6);
    }

    const missing = await adapter.invokeTool(TOY_WORLD_ROLL_DICE_ID, {});
    expect(missing.ok).toBe(false);
    if (!missing.ok) {
      expect(missing.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }

    const zero = await adapter.invokeTool(TOY_WORLD_ROLL_DICE_ID, {
      count: 0,
      sides: 6,
    });
    expect(zero.ok).toBe(false);
    if (!zero.ok) {
      expect(zero.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("lore_lookup returns a seeded knowledge entry read-only", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const result = await adapter.invokeTool(TOY_WORLD_LORE_LOOKUP_ID, {
      entry_id: "kb_tw_mira",
    });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toMatchObject({ entry: { entry_id: "kb_tw_mira" } });
  });

  it("lore_lookup rejects unknown entries and missing entry_id", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();

    const unknown = await adapter.invokeTool(TOY_WORLD_LORE_LOOKUP_ID, {
      entry_id: "kb_tw_nope",
    });
    expect(unknown.ok).toBe(false);
    if (!unknown.ok) {
      expect(unknown.code).toBe(SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND);
    }

    const missing = await adapter.invokeTool(TOY_WORLD_LORE_LOOKUP_ID, {});
    expect(missing.ok).toBe(false);
    if (!missing.ok) {
      expect(missing.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("invokeTool rejects a tool not listed in the manifest (no silent success)", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const result = await adapter.invokeTool("tools.toy_world.snipe", {});

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      // Structured details parity (M2): `{ capability: capability_id }`.
      expect(result.details).toEqual({ capability: "tools.toy_world.snipe" });
    }
  });

  it("rejects a manifest-declared tool with no registered handler (provider bug)", async () => {
    // Explicit empty handler registry: the manifest still declares both
    // frozen tools, but the adapter serves none — the
    // declared-but-unregistered provider-bug state (M1).
    const adapter = new ToyWorldAdapter(undefined, { handlers: new Map() });
    const result = await adapter.invokeTool(TOY_WORLD_ROLL_DICE_ID, {
      count: 1,
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.message).toContain("declared but has no registered handler");
      expect(result.details).toEqual({ capability: TOY_WORLD_ROLL_DICE_ID });
    }
  });

  it("invokeTool rejects a non-tool capability id (grammar gate)", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();
    const result = await adapter.invokeTool("spoke-baseline", {});

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("registerToolHandler overwrites last-wins and rejects non-tool ids", async () => {
    const adapter = ToyWorldAdapter.withCommittedFixtures();

    expect(() =>
      adapter.registerToolHandler(
        "spoke-baseline",
        async () => ({ ok: true as const, value: null }),
      ),
    ).toThrow();

    adapter.registerToolHandler(TOY_WORLD_ROLL_DICE_ID, async (args) => ({
      ok: true as const,
      value: { custom: args },
    }));
    const result = await adapter.invokeTool(TOY_WORLD_ROLL_DICE_ID, {
      count: 1,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual({ custom: { count: 1 } });
    }
  });
});
