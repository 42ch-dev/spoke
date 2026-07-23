import type { ExtensionMap } from "@42ch/spoke-schema";

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function cloneValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => cloneValue(item));
  }

  if (isPlainObject(value)) {
    const cloned: Record<string, unknown> = {};

    for (const [key, nested] of Object.entries(value)) {
      cloned[key] = cloneValue(nested);
    }

    return cloned;
  }

  return value;
}

function cloneNamespace(
  namespace: Record<string, unknown> | undefined,
): Record<string, unknown> {
  if (!namespace) {
    return {};
  }

  return cloneValue(namespace) as Record<string, unknown>;
}

function deepMergeRecords(
  base: Record<string, unknown> | undefined,
  overlay: Record<string, unknown> | undefined,
): Record<string, unknown> {
  const result = cloneNamespace(base);

  if (!overlay) {
    return result;
  }

  for (const [key, overlayValue] of Object.entries(overlay)) {
    const baseValue = result[key];

    if (isPlainObject(baseValue) && isPlainObject(overlayValue)) {
      result[key] = deepMergeRecords(baseValue, overlayValue);
      continue;
    }

    result[key] = cloneValue(overlayValue);
  }

  return result;
}

function mergeExtensionMapsInternal(
  base: ExtensionMap,
  overlay: ExtensionMap,
): ExtensionMap {
  const namespaces = new Set([
    ...Object.keys(base),
    ...Object.keys(overlay),
  ]);
  const result: ExtensionMap = {};

  for (const namespace of namespaces) {
    const merged = deepMergeRecords(base[namespace], overlay[namespace]);
    result[namespace] = merged;
  }

  return result;
}

/**
 * Deep-merge two extension maps; overlay wins on scalar conflicts.
 */
export function mergeExtensionMaps(
  base: ExtensionMap,
  overlay: ExtensionMap,
): ExtensionMap {
  return mergeExtensionMapsInternal(base, overlay);
}

/**
 * Merge maps for round-trip preserve: target wins on known keys;
 * unknown namespaces and keys from source are retained.
 */
export function preserveExtensionMaps(
  source: ExtensionMap,
  target: ExtensionMap,
): ExtensionMap {
  return mergeExtensionMapsInternal(source, target);
}
