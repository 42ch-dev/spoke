import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type { HostCapabilityManifest } from "@42ch/spoke-schemas";

import { capabilitySatisfies, requiredCapability, reservedPrefixOp } from "../src/core/dispatch.js";
import { selectPeerForOp, type SelectablePeer } from "../src/remote/multi-peer-router.js";
import { PORT_OP_CAPABILITY_REQUIREMENTS, portDispatchAllowed } from "../src/remote/responder.js";

interface CapabilityCase {
  op: string;
  table: "core" | "product";
  negotiated: string[];
  required: string;
  allowed: boolean;
}

/** A request-scope verdict case executed through the responder by both harnesses. */
interface RequestScopeCase {
  name: string;
  op: string;
  payload: Record<string, unknown>;
  negotiated: string[];
  admitted_scope: { scope_id: string };
  expected: { allowed?: boolean; code?: string };
}

const MATRIX = JSON.parse(
  readFileSync(fileURLToPath(new URL("./fixtures/capability-matrix.json", import.meta.url)), "utf8"),
) as {
  schema_version: number;
  cases: CapabilityCase[];
  request_scope_cases: RequestScopeCase[];
};

const TS_FIXTURE_BYTES = readFileSync(
  fileURLToPath(new URL("./fixtures/capability-matrix.json", import.meta.url)),
);
const RUST_FIXTURE_BYTES = readFileSync(
  fileURLToPath(
    new URL(
      "../../../crates/spoke-connect/tests/fixtures/capability-matrix.json",
      import.meta.url,
    ),
  ),
);

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
  it("is byte-identical to the Rust fixture", () => {
    expect(TS_FIXTURE_BYTES.equals(RUST_FIXTURE_BYTES)).toBe(true);
  });

  it("carries schema_version 1 rows", () => {
    expect(MATRIX.schema_version).toBe(1);
    expect(MATRIX.cases.length).toBeGreaterThan(0);
  });

  it("covers the scoped request contract on both knowledge ops", () => {
    const cases = MATRIX.request_scope_cases;
    expect(cases.length).toBeGreaterThan(0);
    for (const row of cases) {
      expect(row.negotiated, row.name).toBeInstanceOf(Array);
      expect(
        row.expected.allowed === true || typeof row.expected.code === "string",
        row.name,
      ).toBe(true);
    }
    // Both ops and both marker states are exercised, and every A2 refusal
    // class appears — the responder loops in either stack assert the verdicts.
    expect(new Set(cases.map((row) => row.op))).toEqual(
      new Set(["port.knowledge.get", "port.knowledge.put"]),
    );
    expect(
      cases.some((row) => row.negotiated.includes("spoke-knowledge-scoped")),
    ).toBe(true);
    expect(
      cases.some((row) => !row.negotiated.includes("spoke-knowledge-scoped")),
    ).toBe(true);
    const codes = new Set(cases.map((row) => row.expected.code ?? ""));
    for (const code of [
      "op_unsupported",
      "NOT_AUTHORIZED",
      "INVALID_INPUT",
      "capability_missing",
    ]) {
      expect(codes, code).toContain(code);
    }
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

      expect(
        portDispatchAllowed(row.op, row.negotiated),
        `${row.op}: serve gate for negotiated [${row.negotiated.join(", ")}]`,
      ).toBe(row.allowed);
      if (row.op.startsWith("port.")) {
        expect(requiredCapability(row.op), `${row.op}: core table must not claim a port.* op`).toBe(
          undefined,
        );
      }

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
