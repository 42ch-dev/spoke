import type {
  HostCapabilityManifest,
  ToolDescriptor,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  findTool,
  listTools,
  parseToolCapabilityId,
  SpokeRejectCode,
  toolCapabilityId,
  validateManifestTools,
  validateToolArguments,
  validateToolDescriptor,
} from "@42ch/spoke-operations";

function makeDescriptor(
  overrides: Partial<ToolDescriptor> = {},
): ToolDescriptor {
  return {
    schema_version: 1,
    capability_id: "tools.tool_demo.lookup",
    op: "tools.tool_demo.lookup",
    description: "Look up a knowledge entry by id.",
    input: {},
    output: {},
    ...overrides,
  };
}

function makeManifest(
  tools: ToolDescriptor[] = [],
  namespaces: [string, ...string[]] = ["tool_demo"],
  capabilities: [string, ...string[]] = [
    "spoke-baseline",
    "tools.tool_demo.lookup",
  ],
): HostCapabilityManifest {
  return {
    schema_version: 1,
    host_id: "host-1",
    roles: ["data-store"],
    capabilities,
    namespaces,
    extensions: {},
    tools,
  };
}

function makeManifestWithoutTools(): HostCapabilityManifest {
  const { tools: _omitted, ...rest } = makeManifest();
  return rest;
}

describe("toolCapabilityId", () => {
  it("composes namespace and tool id into a capability id", () => {
    expect(toolCapabilityId("tool_demo", "lookup")).toBe(
      "tools.tool_demo.lookup",
    );
  });

  it("accepts digits and separators in both segments", () => {
    expect(toolCapabilityId("ns_2", "tool-1")).toBe("tools.ns_2.tool-1");
    expect(toolCapabilityId("ns", "0abc")).toBe("tools.ns.0abc");
  });

  it("throws on invalid namespace grammar (programmer misuse)", () => {
    expect(() => toolCapabilityId("Tool Demo", "lookup")).toThrow();
    expect(() => toolCapabilityId("9ns", "lookup")).toThrow();
  });

  it("throws on invalid tool id grammar (programmer misuse)", () => {
    expect(() => toolCapabilityId("tool_demo", "-tool")).toThrow();
    expect(() => toolCapabilityId("tool_demo", "tool!")).toThrow();
  });
});

describe("parseToolCapabilityId", () => {
  it("parses a valid capability id into namespace and tool id", () => {
    expect(parseToolCapabilityId("tools.tool_demo.lookup")).toEqual({
      ok: true,
      value: { namespace: "tool_demo", toolId: "lookup" },
    });
  });

  it("rejects trailing line terminators (ECMA-262 $ is strict end-of-input)", () => {
    // Schema-pattern identity (F-007): the pattern string is byte-identical
    // to the schema. ECMA-262 `$` without the `m` flag anchors at the
    // absolute end of input — a trailing line terminator does NOT match
    // (unlike PCRE/Python `re`; verified against V8 and regress). "Fixing"
    // this to accept trailing terminators would diverge from the schema
    // pattern, which is forbidden.
    expect(parseToolCapabilityId("tools.tool_demo.lookup\n").ok).toBe(false);
    expect(parseToolCapabilityId("tools.tool_demo.lookup\r").ok).toBe(false);
    expect(parseToolCapabilityId("tools.tool_demo.lookup\r\n").ok).toBe(false);
  });

  it("rejects an id without the tools. prefix", () => {
    const result = parseToolCapabilityId("nottools.tool_demo.lookup");
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({
        capability_id: "nottools.tool_demo.lookup",
      });
    }
  });

  it("rejects bad grammar", () => {
    for (const bad of [
      "tools.Tool_demo.lookup",
      "tools..lookup",
      "tools.tool_demo.",
      "tools.tool_demo.too long",
      "tools.9ns.lookup",
      "",
    ]) {
      const result = parseToolCapabilityId(bad);
      expect(result.ok).toBe(false);
      if (result.ok === false) {
        expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      }
    }
  });
});

describe("validateToolDescriptor", () => {
  it("accepts a valid descriptor with an unconstrained ABI", () => {
    expect(validateToolDescriptor(makeDescriptor()).ok).toBe(true);
  });

  it("accepts a descriptor with object-typed input/output subschemas", () => {
    const descriptor = makeDescriptor({
      input: {
        type: "object",
        properties: { query: { type: "string" } },
        required: ["query"],
      },
      output: {
        type: "object",
        properties: {
          ranked_ids: { type: "array", items: { type: "string" } },
        },
        required: ["ranked_ids"],
      },
      idempotent: true,
    });
    expect(validateToolDescriptor(descriptor).ok).toBe(true);
  });

  it("rejects a bad capability_id pattern", () => {
    const result = validateToolDescriptor(
      makeDescriptor({ capability_id: "tools.Bad.lookup" }),
    );
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toMatchObject({
        field: "capability_id",
        capability_id: "tools.Bad.lookup",
      });
    }
  });

  it("rejects a bad op pattern", () => {
    const result = validateToolDescriptor(
      makeDescriptor({ op: "tools.tool_demo.bad op" }),
    );
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toMatchObject({ field: "op" });
    }
  });

  it("rejects op !== capability_id", () => {
    const result = validateToolDescriptor(
      makeDescriptor({ op: "tools.tool_demo.rank" }),
    );
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({
        field: "op",
        op: "tools.tool_demo.rank",
        capability_id: "tools.tool_demo.lookup",
      });
    }
  });

  it("rejects non-object input at the JS boundary", () => {
    const result = validateToolDescriptor(
      makeDescriptor({
        input: "nope" as unknown as ToolDescriptor["input"],
      }),
    );
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({ field: "input" });
    }
  });

  it("rejects non-object output at the JS boundary", () => {
    const result = validateToolDescriptor(
      makeDescriptor({
        output: ["nope"] as unknown as ToolDescriptor["output"],
      }),
    );
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({ field: "output" });
    }
  });
});

describe("validateManifestTools", () => {
  it("accepts a well-formed manifest with consistent tools", () => {
    const manifest = makeManifest(
      [makeDescriptor()],
      ["tool_demo"],
      ["spoke-baseline", "tools.tool_demo.lookup"],
    );
    expect(validateManifestTools(manifest).ok).toBe(true);
  });

  it("accepts a manifest without tools", () => {
    expect(validateManifestTools(makeManifest([])).ok).toBe(true);
    expect(validateManifestTools(makeManifestWithoutTools()).ok).toBe(true);
  });

  it("rejects a descriptor-invalid tool with field + index details", () => {
    const manifest = makeManifest(
      [makeDescriptor({ op: "tools.other.rank" })],
      ["tool_demo"],
      ["spoke-baseline", "tools.tool_demo.lookup"],
    );
    const result = validateManifestTools(manifest);
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toMatchObject({
        field: "tools",
        index: 0,
        capability_id: "tools.tool_demo.lookup",
      });
    }
  });

  it("rejects a tool whose capability_id is missing from capabilities[]", () => {
    const manifest = makeManifest(
      [makeDescriptor()],
      ["tool_demo"],
      ["spoke-baseline"],
    );
    const result = validateManifestTools(manifest);
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({
        field: "tools",
        index: 0,
        capability_id: "tools.tool_demo.lookup",
      });
    }
  });

  it("rejects a tool whose namespace is not owned by the manifest", () => {
    const manifest = makeManifest(
      [makeDescriptor()],
      ["other_ns"],
      ["spoke-baseline", "tools.tool_demo.lookup"],
    );
    const result = validateManifestTools(manifest);
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({
        field: "tools",
        index: 0,
        namespace: "tool_demo",
        capability_id: "tools.tool_demo.lookup",
      });
    }
  });

  it("rejects duplicate capability ids with duplicate_of details", () => {
    const first = makeDescriptor();
    const second = makeDescriptor({
      description: "A second descriptor for the same tool.",
    });
    const manifest = makeManifest(
      [first, second],
      ["tool_demo"],
      ["spoke-baseline", "tools.tool_demo.lookup"],
    );
    const result = validateManifestTools(manifest);
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({
        field: "tools",
        index: 1,
        capability_id: "tools.tool_demo.lookup",
        duplicate_of: 0,
      });
    }
  });

  it("rejects a manifest missing capabilities and namespaces at the JS boundary", () => {
    const {
      capabilities: _capabilities,
      namespaces: _namespaces,
      ...rest
    } = makeManifest();
    const result = validateManifestTools(
      rest as unknown as HostCapabilityManifest,
    );
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({ field: "capabilities" });
    }
  });

  it("rejects a manifest missing namespaces at the JS boundary", () => {
    const { namespaces: _namespaces, ...rest } = makeManifest();
    const result = validateManifestTools(
      rest as unknown as HostCapabilityManifest,
    );
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({ field: "namespaces" });
    }
  });

  it("rejects non-array capabilities and namespaces at the JS boundary", () => {
    const nonArrayCapabilities = makeManifest() as unknown as {
      capabilities?: unknown;
    };
    nonArrayCapabilities.capabilities = "spoke-baseline";
    const capabilitiesResult = validateManifestTools(
      nonArrayCapabilities as unknown as HostCapabilityManifest,
    );
    expect(capabilitiesResult.ok).toBe(false);
    if (capabilitiesResult.ok === false) {
      expect(capabilitiesResult.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(capabilitiesResult.details).toEqual({ field: "capabilities" });
    }

    const nonArrayNamespaces = makeManifest() as unknown as {
      namespaces?: unknown;
    };
    nonArrayNamespaces.namespaces = "tool_demo";
    const namespacesResult = validateManifestTools(
      nonArrayNamespaces as unknown as HostCapabilityManifest,
    );
    expect(namespacesResult.ok).toBe(false);
    if (namespacesResult.ok === false) {
      expect(namespacesResult.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(namespacesResult.details).toEqual({ field: "namespaces" });
    }
  });
});

describe("validateToolArguments", () => {
  it("passes any object arguments when input is unconstrained", () => {
    expect(validateToolArguments(makeDescriptor(), {}).ok).toBe(true);
    expect(validateToolArguments(makeDescriptor(), { any: "thing" }).ok).toBe(
      true,
    );
  });

  it("rejects non-object arguments with field details", () => {
    for (const bad of [null, 42, "nope", ["a"]]) {
      const result = validateToolArguments(
        makeDescriptor(),
        bad as unknown as Record<string, unknown>,
      );
      expect(result.ok).toBe(false);
      if (result.ok === false) {
        expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
        expect(result.details).toEqual({ field: "arguments" });
      }
    }
  });

  it("accepts arguments satisfying the required keys", () => {
    const descriptor = makeDescriptor({
      input: {
        type: "object",
        properties: {
          query: { type: "string" },
          limit: { type: "integer" },
        },
        required: ["query"],
      },
    });
    expect(validateToolArguments(descriptor, { query: "x", limit: 3 }).ok).toBe(
      true,
    );
    expect(validateToolArguments(descriptor, { query: "x" }).ok).toBe(true);
  });

  it("rejects missing required keys listing them in details", () => {
    const descriptor = makeDescriptor({
      input: {
        type: "object",
        properties: {
          query: { type: "string" },
          limit: { type: "integer" },
        },
        required: ["query", "limit"],
      },
    });
    const result = validateToolArguments(descriptor, { query: "x" });
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({
        field: "arguments",
        missing: ["limit"],
      });
    }
    const allMissing = validateToolArguments(descriptor, {});
    expect(allMissing.ok).toBe(false);
    if (allMissing.ok === false) {
      expect(allMissing.details).toEqual({
        field: "arguments",
        missing: ["query", "limit"],
      });
    }
  });

  it("skips the required-keys gate when input.type is not object", () => {
    const descriptor = makeDescriptor({ input: { type: "string" } });
    expect(validateToolArguments(descriptor, {}).ok).toBe(true);
  });

  it("rejects a runtime non-object descriptor.input at the JS boundary", () => {
    // JS boundary: `input: null` from unvalidated JSON must reject with the
    // same object-ness guard as validateToolDescriptor, not throw a TypeError.
    const descriptor = makeDescriptor({
      input: null as unknown as ToolDescriptor["input"],
    });
    const result = validateToolArguments(descriptor, {});
    expect(result.ok).toBe(false);
    if (result.ok === false) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toEqual({ field: "input" });
    }
  });

  it("skips the required-keys gate when required is not an array", () => {
    const descriptor = makeDescriptor({
      input: { type: "object", required: "query" },
    });
    expect(validateToolArguments(descriptor, {}).ok).toBe(true);
  });
});

describe("listTools", () => {
  it("returns tools in manifest order", () => {
    const tools = [
      makeDescriptor({
        capability_id: "tools.tool_demo.alpha",
        op: "tools.tool_demo.alpha",
      }),
      makeDescriptor({
        capability_id: "tools.tool_demo.beta",
        op: "tools.tool_demo.beta",
      }),
    ];
    expect(listTools(makeManifest(tools))).toEqual(tools);
  });

  it("returns an empty array when tools is absent", () => {
    expect(listTools(makeManifestWithoutTools())).toEqual([]);
  });

  it("returns a defensive copy (mutating the result does not mutate the manifest)", () => {
    const tools = [makeDescriptor()];
    const manifest = makeManifest(tools);

    const listed = listTools(manifest);
    listed.push(
      makeDescriptor({
        capability_id: "tools.tool_demo.rank",
        op: "tools.tool_demo.rank",
      }),
    );

    expect(listed).toHaveLength(2);
    expect(manifest.tools).toEqual(tools);
  });
});

describe("findTool", () => {
  it("returns the matching descriptor with ABI intact", () => {
    const descriptor = makeDescriptor({
      input: { type: "object", required: ["query"] },
      output: {},
    });
    const found = findTool(
      makeManifest([descriptor]),
      "tools.tool_demo.lookup",
    );
    expect(found).toEqual(descriptor);
  });

  it("returns undefined when no descriptor matches", () => {
    expect(
      findTool(makeManifest([makeDescriptor()]), "tools.other.nope"),
    ).toBeUndefined();
  });
});
