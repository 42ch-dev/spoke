// Regression test for hoistDeepRefs name allocation (QC fix wave, F1).
// An existing definition named "Shared0" must survive; the hoisted target
// receives the first absent synthesized name.
import { test } from "node:test";
import assert from "node:assert/strict";
import { hoistDeepRefs } from "../src/deep-ref-hoist.mjs";

test("hoist allocates the first absent SharedN without overwriting existing definitions", () => {
  const deepTarget = { type: "integer", minimum: 1, title: "DeepTarget" };
  const doc = {
    $schema: "http://json-schema.org/draft-07/schema#",
    $id: "https://spoke42.invalid/schemas/ops/hoist-fixture.schema.json",
    title: "HoistFixture",
    type: "object",
    properties: {
      first: { $ref: "#/definitions/Holder/properties/deep" },
      second: { $ref: "#/definitions/Holder/properties/deep" },
    },
    required: ["first", "second"],
    additionalProperties: false,
    definitions: {
      // Pre-existing synthesized-style name: must never be overwritten.
      Shared0: { type: "string", minLength: 1, title: "PreexistingShared0" },
      Holder: {
        type: "object",
        properties: { deep: deepTarget },
        additionalProperties: false,
      },
    },
  };

  hoistDeepRefs(doc);

  // Original definition untouched.
  assert.deepEqual(doc.definitions.Shared0, {
    type: "string",
    minLength: 1,
    title: "PreexistingShared0",
  });
  // Deep target still reachable at its original location.
  assert.deepEqual(doc.definitions.Holder.properties.deep, deepTarget);
  // Both refs hoisted to the SAME new definition (content dedup), named Shared1
  // (first absent after the pre-existing Shared0).
  assert.equal(doc.properties.first.$ref, "#/definitions/Shared1");
  assert.equal(doc.properties.second.$ref, "#/definitions/Shared1");
  assert.deepEqual(doc.definitions.Shared1, deepTarget);
  assert.equal(doc.definitions.Shared2, undefined);
});
