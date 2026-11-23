import { describe, expect, it } from "vitest";

import type { KnowledgeEntry } from "@42ch/spoke-schemas";

import { assertAdoptionAnchorPreserved } from "./adoption.js";

function entry(entryId: string, adoptionAnchor?: unknown): KnowledgeEntry {
  const wire: Record<string, unknown> = {
    schema_version: 1,
    entry_id: entryId,
    entry_type: "character",
    canonical_name: "Mira Vale",
    status: "confirmed",
    body: {},
    extensions: {},
  };
  if (adoptionAnchor !== undefined) {
    wire.adoption_anchor = adoptionAnchor;
  }
  return wire as unknown as KnowledgeEntry;
}

function anchor(sourceRevision: number): Record<string, unknown> {
  return {
    schema_version: 1,
    source_entry_id: "ke_terminal",
    source_scope_id: "scope_a",
    source_revision: sourceRevision,
    extensions: {},
  };
}

describe("assertAdoptionAnchorPreserved", () => {
  it("passes an identical anchor", () => {
    const stored = entry("ke_dst", anchor(3));
    const candidate = entry("ke_dst", anchor(3));
    expect(assertAdoptionAnchorPreserved(candidate, stored)).toEqual({
      ok: true,
    });
  });

  it("refuses a changed anchor as ADOPTION_ANCHOR_IMMUTABLE", () => {
    const stored = entry("ke_dst", anchor(3));
    const candidate = entry("ke_dst", anchor(4));
    expect(assertAdoptionAnchorPreserved(candidate, stored)).toMatchObject({
      ok: false,
      code: "ADOPTION_ANCHOR_IMMUTABLE",
      details: { entry_id: "ke_dst" },
    });
  });

  it("refuses a removed anchor as ADOPTION_ANCHOR_IMMUTABLE", () => {
    const stored = entry("ke_dst", anchor(3));
    const candidate = entry("ke_dst");
    expect(assertAdoptionAnchorPreserved(candidate, stored)).toMatchObject({
      ok: false,
      code: "ADOPTION_ANCHOR_IMMUTABLE",
    });
  });

  it("refuses INVALID_INPUT when either anchor fails canonicalization", () => {
    // In-process providers hand over object graphs, not wire-decoded values:
    // a cyclic extensions member is uncanonicalizable. Regression pin —
    // `undefined === undefined` used to pass two failed canonicalizations
    // as "unchanged".
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    const stored = entry("ke_dst", anchor(3));
    const uncanonicalizable = entry("ke_dst", {
      ...anchor(3),
      extensions: cyclic,
    });
    expect(
      assertAdoptionAnchorPreserved(uncanonicalizable, stored),
    ).toMatchObject({ ok: false, code: "INVALID_INPUT" });

    // Both sides failing is a refusal, never a match.
    const storedCyclic = entry("ke_dst", {
      ...anchor(3),
      extensions: cyclic,
    });
    expect(
      assertAdoptionAnchorPreserved(uncanonicalizable, storedCyclic),
    ).toMatchObject({ ok: false, code: "INVALID_INPUT" });

    // A changed anchor that still fails refuses INVALID_INPUT, not a pass.
    const mutated = entry("ke_dst", {
      ...anchor(4),
      extensions: cyclic,
    });
    expect(
      assertAdoptionAnchorPreserved(mutated, storedCyclic),
    ).toMatchObject({ ok: false, code: "INVALID_INPUT" });
  });
});
