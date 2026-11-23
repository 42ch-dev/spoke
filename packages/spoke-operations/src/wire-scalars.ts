/**
 * Shared wire scalar domains (spoke-data-model §Shared wire scalars).
 *
 * `SchemaVersion` is `[1, 2^53-1]` — the largest value both stacks represent
 * exactly — and every revision-typed integer is `[0, 2^53]` inclusive, one
 * step beyond `Number.isSafeInteger` (`2^53 + 1` is not representable in
 * IEEE-754 and rounds to `2^53`). The bound is asserted on the parsed value.
 */
export const MAX_SCHEMA_VERSION = 2 ** 53 - 1;
export const MAX_REVISION = 2 ** 53;

export function isSchemaVersion(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 1
  );
}

export function isRevisionInteger(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isInteger(value) &&
    value >= 0 &&
    value <= MAX_REVISION
  );
}
