import { describe, expect, it } from "vitest";

import type { RevisionSnapshot } from "@42ch/spoke-schemas";

import {
  revisionSnapshotsEqualCanonical,
  type HistoryPruneResult,
} from "./history.js";

function snapshot(): RevisionSnapshot {
  return {
    schema_version: 1,
    entity_kind: "knowledge_entry",
    entity_id: "ke_mira_harbor",
    revision: 3,
    snapshot: {
      schema_version: 1,
      entry_id: "ke_mira_harbor",
      entry_type: "character",
      canonical_name: "Mira",
      status: "confirmed",
      body: {},
      extensions: {},
    } as RevisionSnapshot["snapshot"],
    extensions: {},
  };
}

describe("revisionSnapshotsEqualCanonical", () => {
  it("identical snapshots compare equal", () => {
    const result = revisionSnapshotsEqualCanonical(snapshot(), snapshot());
    expect(result).toEqual({ ok: true, value: true });
  });

  it("different revisions compare unequal", () => {
    const other = snapshot();
    other.revision = 4;
    const result = revisionSnapshotsEqualCanonical(snapshot(), other);
    expect(result).toEqual({ ok: true, value: false });
  });

  it("member order does not change the canonical bytes", () => {
    // The same document written with members in a different order has the
    // same RFC 8785 bytes — this is the property the helper exists to assert.
    const reordered = JSON.parse(
      JSON.stringify({
        extensions: {},
        snapshot: {
          extensions: {},
          body: {},
          status: "confirmed",
          canonical_name: "Mira",
          entry_type: "character",
          entry_id: "ke_mira_harbor",
          schema_version: 1,
        },
        revision: 3,
        entity_id: "ke_mira_harbor",
        entity_kind: "knowledge_entry",
        schema_version: 1,
      }),
    ) as RevisionSnapshot;
    const result = revisionSnapshotsEqualCanonical(snapshot(), reordered);
    expect(result).toEqual({ ok: true, value: true });
  });
});

describe("revision snapshot wire presence", () => {
  it("explicitly present-and-empty members change canonical bytes", () => {
    // note: product-behavior assertion — the wire-presence rule (contract
    // §RevisionSnapshot): `body.tags: []`, `body.state: {}`, `modules: {}`
    // and `body.attributes: []` are byte-significant under RFC 8785, so a
    // snapshot carrying them MUST NOT compare equal to one where they are
    // absent. Guards against a representation that drops present-empty
    // members before canonicalization (qc1-F-005).
    const empty: RevisionSnapshot = snapshot();
    empty.snapshot = {
      ...(empty.snapshot as object),
      body: { tags: [], state: {}, attributes: [] },
      modules: {},
    } as RevisionSnapshot["snapshot"];
    expect(revisionSnapshotsEqualCanonical(snapshot(), empty)).toEqual({
      ok: true,
      value: false,
    });

    const absentThenPresent = snapshot();
    absentThenPresent.snapshot = {
      ...(absentThenPresent.snapshot as object),
      modules: {},
    } as RevisionSnapshot["snapshot"];
    expect(
      revisionSnapshotsEqualCanonical(empty, absentThenPresent),
    ).toEqual({ ok: true, value: false });

    // note: a snapshot identical including the empty members still
    // canonicalizes equal — presence is preserved, not invented.
    const alsoEmpty = snapshot();
    alsoEmpty.snapshot = JSON.parse(JSON.stringify(empty.snapshot));
    expect(revisionSnapshotsEqualCanonical(empty, alsoEmpty)).toEqual({
      ok: true,
      value: true,
    });
  });

  it("HistoryPruneResult carries deleted + nullable floor_revision (contract §4.2)", () => {
    // note: the prune outcome vocabulary — a deleting prune reports the
    // retained floor; a no-op reports `null`, never an omitted member.
    const deleted: HistoryPruneResult = { deleted: 1, floor_revision: 2 };
    const noop: HistoryPruneResult = { deleted: 0, floor_revision: null };
    expect("floor_revision" in noop).toBe(true);
    expect(noop.floor_revision).toBeNull();
    expect(deleted.deleted).toBeGreaterThanOrEqual(0);
  });
});
