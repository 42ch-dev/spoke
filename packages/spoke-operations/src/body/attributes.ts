import type { BodyAttribute, KnowledgeEntry } from "@42ch/spoke-schemas";

export type BodyAttributesInput =
  | KnowledgeEntry["body"]
  | KnowledgeEntry
  | null
  | undefined;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function isBodyAttributeValue(
  value: unknown,
): value is BodyAttribute["value"] {
  return (
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  );
}

function parseBodyAttribute(value: unknown): BodyAttribute | undefined {
  if (!isPlainObject(value)) {
    return undefined;
  }

  const traitType = value.trait_type;
  const traitValue = value.value;

  if (!isNonEmptyString(traitType) || !isBodyAttributeValue(traitValue)) {
    return undefined;
  }

  const attribute: BodyAttribute = {
    trait_type: traitType,
    value: traitValue,
  };

  if (typeof value.display_type === "string") {
    attribute.display_type = value.display_type;
  }

  if (typeof value.max_value === "number") {
    attribute.max_value = value.max_value;
  }

  return attribute;
}

function extractAttributesArray(input: BodyAttributesInput): unknown[] {
  if (input == null) {
    return [];
  }

  if (!isPlainObject(input)) {
    return [];
  }

  const body =
    "entry_id" in input && "body" in input
      ? (input as KnowledgeEntry).body
      : (input as KnowledgeEntry["body"]);

  if (!isPlainObject(body)) {
    return [];
  }

  const { attributes } = body;

  if (!Array.isArray(attributes)) {
    return [];
  }

  return attributes;
}

/**
 * Lists valid body.attributes traits in array order.
 * Accepts a KnowledgeEntry body or full entry. Missing input yields [].
 */
export function listBodyAttributes(
  input: BodyAttributesInput,
): BodyAttribute[] {
  const result: BodyAttribute[] = [];

  for (const element of extractAttributesArray(input)) {
    const parsed = parseBodyAttribute(element);
    if (parsed !== undefined) {
      result.push(parsed);
    }
  }

  return result;
}

/**
 * Returns all traits with the given trait_type in original array order.
 */
export function filterBodyAttributesByTraitType(
  input: BodyAttributesInput,
  traitType: string,
): BodyAttribute[] {
  return listBodyAttributes(input).filter(
    (attribute) => attribute.trait_type === traitType,
  );
}

/**
 * Returns the first trait with the given trait_type, or undefined.
 */
export function findBodyAttribute(
  input: BodyAttributesInput,
  traitType: string,
): BodyAttribute | undefined {
  return filterBodyAttributesByTraitType(input, traitType)[0];
}
