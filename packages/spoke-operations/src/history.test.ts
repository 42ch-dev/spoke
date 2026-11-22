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

describe("HistoryPruneResult", () => {
  it("serializes to the wire shape", () => {
    const deleted: HistoryPruneResult = { deleted: 1, floor_revision: 2 };
    expect(JSON.parse(JSON.stringify(deleted))).toEqual({
      deleted: 1,
      floor_revision: 2,
    });
    const noop: HistoryPruneResult = { deleted: 0, floor_revision: null };
    expect(JSON.parse(JSON.stringify(noop))).toEqual({
      deleted: 0,
      floor_revision: null,
    });
  });
});
