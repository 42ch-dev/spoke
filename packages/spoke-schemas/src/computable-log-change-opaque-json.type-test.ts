/**
 * Type-level evidence: ComputableLogChange.previous / .next accept any JSON value.
 * Fails typecheck until codegen regenerates types from OpaqueJson schema (Task 2).
 */
import type { ComputableLogChange } from "./generated/common/common";

// Scalar previous
const _scalarPrevious: ComputableLogChange = {
  path: "level",
  previous: 2.1,
};

// Null next
const _nullNext: ComputableLogChange = {
  path: "level",
  next: null,
};

// Array next
const _arrayNext: ComputableLogChange = {
  path: "tags",
  next: ["x"],
};
