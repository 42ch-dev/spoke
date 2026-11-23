import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type { HostCapabilityManifest } from "@42ch/spoke-schemas";

import { capabilitySatisfies, requiredCapability, reservedPrefixOp } from "../src/core/dispatch.js";
import { selectPeerForOp, type SelectablePeer } from "../src/remote/multi-peer-router.js";
import { PORT_OP_CAPABILITY_REQUIREMENTS, portDispatchAllowed } from "../src/remote/responder.js";

/**
 * why: capability selection matrix parity. The fixture is a byte-identical
 * registered copy of the SSOT under
 * `crates/spoke-connect/tests/fixtures/capability-matrix.json` (sync gate:
 * `tooling/connect/golden-vector-sync.mjs`); the Rust twin iterates the same
 * rows against its own tables, so a requirement or verdict drift on either
 * side fails here or there.
 */
interface CapabilityCase {
  op: string;
  table: "core" | "product";
  negotiated: string[];
  required: string;
  allowed: boolean;
}

const MATRIX = JSON.parse(
  readFileSync(fileURLToPath(new URL("./fixtures/capability-matrix.json", import.meta.url)), "utf8"),
) as { schema_version: number; cases: CapabilityCase[] };

function tableRequirement(row: CapabilityCase): string | undefined {
  return row.table === "core"
    ? requiredCapability(row.op)
    : PORT_OP_CAPABILITY_REQUIREMENTS[row.op];
}

function payloadFor(op: string): Record<string, unknown> {
  switch (op) {
    case "port.scope.list_knowledge_entries":
    case "port.scope.list_timeline_events":
      return { scope: { scope_id: "s" } };
    case "port.knowledge.get":
      return { entry_id: "e" };
    case "port.knowledge.put":
      return { entry: {} };
    default:
      return {};
  }
}

function candidate(negotiated: string[]): SelectablePeer {
  const manifest: HostCapabilityManifest = {
    schema_version: 1,
    host_id: "h-matrix",
    roles: ["data-store"],
    capabilities: negotiated as [string, ...string[]],
    namespaces: ["ns"],
    extensions: {},
  };
  return { peerId: "peer-matrix", manifest };
}

describe("shared capability matrix", () => {
  it("carries schema_version 1 rows", () => {
    expect(MATRIX.schema_version).toBe(1);
    expect(MATRIX.cases.length).toBeGreaterThan(0);
  });

  it("matches the core and product tables row by row", () => {
    for (const row of MATRIX.cases) {
      expect(tableRequirement(row), `${row.op}: ${row.table} table requirement`).toBe(
        row.required,
      );
      expect(
        capabilitySatisfies(row.negotiated, row.required),
        `${row.op}: verdict for negotiated [${row.negotiated.join(", ")}]`,
      ).toBe(row.allowed);

      // why: the responder's actual combined gate, not just the row's owning
      // table: a core-table entry for a `port.*` op would otherwise pass both
      // lookups above while admitting the wrong session.
      expect(
        portDispatchAllowed(row.op, row.negotiated),
        `${row.op}: serve gate for negotiated [${row.negotiated.join(", ")}]`,
      ).toBe(row.allowed);
      if (row.op.startsWith("port.")) {
        expect(requiredCapability(row.op), `${row.op}: core table must not claim a port.* op`).toBe(
          undefined,
        );
      }

      // why: the real selection path, not just the table lookup: one candidate
      // carrying exactly this row's negotiated set must be selected for an
      // allowed row and rejected for a denied one.
      const selection = selectPeerForOp([candidate(row.negotiated)], row.op, payloadFor(row.op));
      if (row.allowed) {
        expect(selection.ok, `${row.op}: router must select the row's peer`).toBe(true);
        if (selection.ok) {
          expect(selection.value.peerId, `${row.op}: router selected an unexpected peer`).toBe(
            "peer-matrix",
          );
        }
      } else {
        expect(
          selection.ok,
          `${row.op}: router must reject negotiated [${row.negotiated.join(", ")}]`,
        ).toBe(false);
      }
    }
  });
});

describe("reserved-prefix terminal dispatch (QC2-16)", () => {
  it("a reserved knowledge.* unknown is terminal before the product map", () => {
    // note: the shipped product map is constant, so a conflicting row is
    // pinned by the guard's contract: `reservedPrefixOp` is the SAME
    // predicate the gate runs before its product-map fallback — a
    // reserved unknown denies even when a map row + negotiated capability
    // would otherwise admit it.
    for (const op of ["knowledge.archive", "knowledge.", "knowledge.migrate"]) {
      expect(reservedPrefixOp(op), `${op}: reserved unknown`).toBe(true);
      expect(
        portDispatchAllowed(op, [
          "spoke-baseline",
          "spoke-knowledge",
          "spoke-knowledge-adoption",
        ]),
        `${op}: serve gate must refuse before the product map`,
      ).toBe(false);
    }
    expect(reservedPrefixOp("knowledge.adopt")).toBe(false);
  });

  it("a conflicting product-map row never advances a reserved unknown (F-011)", () => {
    // note: the conflicting row IS the distinguishing case: removing the
    // production gate's reserved short-circuit would let this map admit
    // the op — the shipped-map-only checks above cannot tell old from new.
    const conflictingMap = {
      "knowledge.made_up": "spoke-knowledge",
    } as const;
    expect(
      portDispatchAllowed(
        "knowledge.made_up",
        ["spoke-baseline", "spoke-knowledge"],
        conflictingMap,
      ),
      "a custom-map row must never authorize a reserved knowledge.* unknown",
    ).toBe(false);
    // And a non-reserved name still resolves through the same injected map —
    // the denial above is the prefix rule, not a broken map lookup.
    expect(
      portDispatchAllowed(
        "port.knowledge.get",
        ["spoke-knowledge"],
        { "port.knowledge.get": "spoke-knowledge" },
      ),
      "an injected map must still serve non-reserved ops",
    ).toBe(true);
  });

  it("a product-map row for the registered knowledge.adopt never bypasses the dual-capability gate (F-020)", () => {
    // note: the shipped map grants knowledge.adopt nothing — an injected
    // row claiming `spoke-baseline` (or even `spoke-knowledge`) must not
    // substitute for the op's OWN dual requirement
    // (`spoke-knowledge-adoption` AND `spoke-knowledge`).
    const conflictingMap = {
      "knowledge.adopt": "spoke-baseline",
    } as const;
    for (const negotiated of [
      ["spoke-baseline"],
      ["spoke-knowledge"],
      ["spoke-knowledge-adoption"],
      ["spoke-baseline", "spoke-knowledge"],
    ] as const) {
      expect(
        portDispatchAllowed("knowledge.adopt", negotiated, conflictingMap),
        `negotiated=${negotiated}: map row must never short-circuit the core requirement`,
      ).toBe(false);
    }
    // The legitimate pair still serves — through the op's own core
    // requirement, not through the injected row.
    expect(
      portDispatchAllowed(
        "knowledge.adopt",
        ["spoke-knowledge-adoption", "spoke-knowledge"],
        conflictingMap,
      ),
    ).toBe(true);
    // And the same gate holds with the shipped map (param default).
    expect(
      portDispatchAllowed("knowledge.adopt", ["spoke-baseline"]),
    ).toBe(false);
  });
});
