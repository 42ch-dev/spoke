import type {
  HostCapabilityManifest,
  ToolDescriptor,
} from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

/**
 * Tool capability grammar: `tools.<ns>.<tool_id>`.
 *
 * The RegExp is byte-identical to the schema pattern in
 * `schemas/data/tool-descriptor.schema.json`
 * (`^tools\.[a-z][a-z0-9_-]*\.[a-z0-9][a-z0-9_-]*$`). It therefore inherits
 * native ECMA-262 semantics: `$` without the `m` flag anchors at the
 * absolute end of input — a trailing line terminator does NOT match (this
 * differs from PCRE/Python `re`, where `$` also matches before a final
 * `\n`; verified against V8 and regress). Do not diverge from the schema
 * pattern string in either direction.
 */
const TOOL_CAPABILITY_PATTERN =
  /^tools\.[a-z][a-z0-9_-]*\.[a-z0-9][a-z0-9_-]*$/;

export type ToolCapabilityId = {
  namespace: string;
  toolId: string;
};

function isJsonObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOwn(object: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(object, key);
}

/**
 * Compose a tool capability id `tools.<ns>.<tool_id>` and assert its grammar.
 *
 * Throws on bad grammar — programmer misuse, consistent with generated-type
 * ergonomics. Never returns an id that `parseToolCapabilityId` would reject.
 */
export function toolCapabilityId(namespace: string, toolId: string): string {
  const id = `tools.${namespace}.${toolId}`;
  if (!TOOL_CAPABILITY_PATTERN.test(id)) {
    throw new Error(
      `Invalid tool capability grammar: "${id}" (expected tools.<ns>.<tool_id> with ns ^[a-z][a-z0-9_-]*$ and tool_id ^[a-z0-9][a-z0-9_-]*$)`,
    );
  }
  return id;
}

/**
 * Parse a tool capability id into its namespace and tool id.
 *
 * Rejects with `INVALID_INPUT` for a missing `tools.` prefix and for bad
 * grammar. The pattern is the schema pattern (see `TOOL_CAPABILITY_PATTERN`);
 * ECMA-262 `$` without the `m` flag is a strict end-of-input anchor, so a
 * trailing line terminator is rejected.
 */
export function parseToolCapabilityId(
  id: string,
): SpokeResult<ToolCapabilityId> {
  if (!id.startsWith("tools.")) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `Tool capability id must start with "tools." prefix: "${id}"`,
      { capability_id: id },
    );
  }
  const match = TOOL_CAPABILITY_PATTERN.exec(id);
  if (match === null) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `Invalid tool capability grammar: "${id}" (expected tools.<ns>.<tool_id> with ns ^[a-z][a-z0-9_-]*$ and tool_id ^[a-z0-9][a-z0-9_-]*$)`,
      { capability_id: id },
    );
  }
  // The pattern guarantees exactly three dot-separated segments; split the
  // matched text for clarity (strict `$` cannot admit a trailing terminator).
  const [, namespace, toolId] = match[0].split(".");
  return spokeOk({ namespace, toolId });
}

/**
 * Validate a `ToolDescriptor` (pattern, `op === capability_id`, `input` /
 * `output` are JSON objects) with `INVALID_INPUT`.
 */
export function validateToolDescriptor(
  descriptor: ToolDescriptor,
): SpokeResult<void> {
  if (!TOOL_CAPABILITY_PATTERN.test(descriptor.capability_id)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `ToolDescriptor capability_id must match pattern ^tools\\.[a-z][a-z0-9_-]*\\.[a-z0-9][a-z0-9_-]*$: "${descriptor.capability_id}"`,
      { field: "capability_id", capability_id: descriptor.capability_id },
    );
  }
  if (!TOOL_CAPABILITY_PATTERN.test(descriptor.op)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `ToolDescriptor op must match pattern ^tools\\.[a-z][a-z0-9_-]*\\.[a-z0-9][a-z0-9_-]*$: "${descriptor.op}"`,
      { field: "op", op: descriptor.op },
    );
  }
  if (descriptor.op !== descriptor.capability_id) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `ToolDescriptor op must equal capability_id: op="${descriptor.op}" capability_id="${descriptor.capability_id}"`,
      {
        field: "op",
        op: descriptor.op,
        capability_id: descriptor.capability_id,
      },
    );
  }
  // Runtime guards for the JS boundary: the generated TS type always declares
  // objects, but a caller can hand us anything from unvalidated JSON.
  if (!isJsonObject(descriptor.input)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ToolDescriptor input must be a JSON object subschema",
      { field: "input" },
    );
  }
  if (!isJsonObject(descriptor.output)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ToolDescriptor output must be a JSON object subschema",
      { field: "output" },
    );
  }
  return spokeOk();
}

/**
 * Validate every tool in a manifest: each descriptor is valid, its
 * `capability_id` appears in `capabilities[]`, its derived namespace is owned
 * (`namespaces[]` membership), and capability ids are unique within
 * `tools[]`. Rejects with `INVALID_INPUT` and structured `details`
 * (`field: "tools"` + `index`). JS-boundary presence guards reject
 * missing/non-array `capabilities`/`namespaces` with `details.field` =
 * `"capabilities"`/`"namespaces"`.
 */
export function validateManifestTools(
  manifest: HostCapabilityManifest,
): SpokeResult<void> {
  // JS-boundary guards (same rationale as validateToolDescriptor's
  // input/output guards): the generated TS type declares these required, but
  // a caller can hand us anything from unvalidated JSON. Missing or
  // non-array fields reject structurally instead of throwing a TypeError.
  if (!Array.isArray(manifest.capabilities)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Manifest capabilities must be an array",
      { field: "capabilities" },
    );
  }
  if (!Array.isArray(manifest.namespaces)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Manifest namespaces must be an array",
      { field: "namespaces" },
    );
  }
  const tools = manifest.tools ?? [];
  const seen = new Map<string, number>();
  for (let index = 0; index < tools.length; index += 1) {
    const descriptor = tools[index];
    const descriptorCheck = validateToolDescriptor(descriptor);
    if (descriptorCheck.ok === false) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        descriptorCheck.message,
        { field: "tools", index, capability_id: descriptor.capability_id },
      );
    }
    if (!manifest.capabilities.includes(descriptor.capability_id)) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `Tool capability "${descriptor.capability_id}" at tools[${index}] is missing from manifest capabilities[]`,
        { field: "tools", index, capability_id: descriptor.capability_id },
      );
    }
    // validateToolDescriptor passed, so the grammar guarantees exactly three
    // dot-separated segments; the split below is total.
    const namespace = descriptor.capability_id.split(".")[1];
    if (!manifest.namespaces.includes(namespace)) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `Tool namespace "${namespace}" at tools[${index}] is not owned by this manifest (namespaces[])`,
        {
          field: "tools",
          index,
          namespace,
          capability_id: descriptor.capability_id,
        },
      );
    }
    const duplicateOf = seen.get(descriptor.capability_id);
    if (duplicateOf !== undefined) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `Duplicate tool capability_id "${descriptor.capability_id}" at tools[${index}] (first declared at tools[${duplicateOf}])`,
        {
          field: "tools",
          index,
          capability_id: descriptor.capability_id,
          duplicate_of: duplicateOf,
        },
      );
    }
    seen.set(descriptor.capability_id, index);
  }
  return spokeOk();
}

/**
 * Structural argument gate (frozen granularity): `args` must be a JSON
 * object; when `descriptor.input` declares top-level `"type": "object"` with
 * `"required": [...]`, each listed string key must be present in `args`.
 * No deeper JSON-Schema checking — full validation stays consumer/fixture-side.
 * `input: {}` ⇒ vacuous (unconstrained).
 *
 * A runtime non-object `descriptor.input` (JS-boundary, unvalidated JSON)
 * REJECTs with `INVALID_INPUT` + `details.field = "input"` — the same
 * object-ness guard `validateToolDescriptor` applies. Chosen over a vacuous
 * pass deliberately: a malformed schema must not silently skip the gate.
 *
 * Named `args` because `arguments` is not a legal binding in strict-mode
 * modules; the contract's second parameter is the tool arguments object.
 */
export function validateToolArguments(
  descriptor: ToolDescriptor,
  args: Record<string, unknown>,
): SpokeResult<void> {
  if (!isJsonObject(args)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Tool arguments must be a JSON object",
      { field: "arguments" },
    );
  }
  // JS-boundary guard: `descriptor.input` may be a runtime non-object from
  // unvalidated JSON. REJECT (INVALID_INPUT + details.field = "input") for
  // consistency with validateToolDescriptor's object-ness guard — deliberately
  // not a vacuous pass, so a malformed schema cannot silently skip the gate.
  if (!isJsonObject(descriptor.input)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ToolDescriptor input must be a JSON object subschema",
      { field: "input" },
    );
  }
  const input = descriptor.input;
  if (input["type"] !== "object" || !Array.isArray(input["required"])) {
    return spokeOk();
  }
  const required = input["required"] as unknown[];
  const missing = required.filter(
    (key): key is string =>
      typeof key === "string" && !hasOwn(args, key),
  );
  if (missing.length > 0) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `Missing required tool arguments: ${missing.join(", ")}`,
      { field: "arguments", missing },
    );
  }
  return spokeOk();
}

/**
 * List the manifest's tools in declaration order (empty when `tools` absent).
 * Returns a defensive copy — mutating the result does not mutate the manifest
 * (parity with Rust `list_tools`, which returns an owned clone).
 */
export function listTools(manifest: HostCapabilityManifest): ToolDescriptor[] {
  return [...(manifest.tools ?? [])];
}

/**
 * Find the tool descriptor whose `capability_id` exactly matches, or
 * `undefined` when no descriptor matches.
 */
export function findTool(
  manifest: HostCapabilityManifest,
  capabilityId: string,
): ToolDescriptor | undefined {
  return (manifest.tools ?? []).find(
    (descriptor) => descriptor.capability_id === capabilityId,
  );
}
