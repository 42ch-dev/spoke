/**
 * note: Type-level evidence — ComputableLogChange.previous / .next accept any JSON value.
 * Asserts scalar, null, and array assignments typecheck against generated OpaqueJson.
 */
import type { ComputableLogChange } from "./generated/common/common";

const _scalarPrevious: ComputableLogChange = {
  path: "level",
  previous: 2.1,
};

const _nullNext: ComputableLogChange = {
  path: "level",
  next: null,
};

const _arrayNext: ComputableLogChange = {
  path: "tags",
  next: ["x"],
};
