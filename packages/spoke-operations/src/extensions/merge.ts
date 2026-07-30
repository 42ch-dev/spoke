import type { ExtensionMap, ModuleMap } from "@42ch/spoke-schemas";

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

/**
 * Merge two structured JSON values: deep-merge when both are plain objects;
 * otherwise the overlay replaces the base (arrays/scalars are not element-merged).
 * When the overlay is absent, the base is retained. Shared primitive for the
 * generalized namespace merge below.
 */
function mergeJsonValues(base: unknown, overlay: unknown): unknown {
  if (isPlainObject(base) && isPlainObject(overlay)) {
    return deepMergeRecords(base, overlay);
  }

  return overlay !== undefined ? cloneValue(overlay) : cloneValue(base);
}

/**
 * Generalized namespace merge: iterate the union of namespace keys and merge
 * each value via `mergeJsonValues` (object deep-merge; arrays/other replaced by
 * overlay). Shared core for extension and module map helpers — no duplicate
 * merge logic.
 */
function mergeNamespaceMaps(
  base: Record<string, unknown>,
  overlay: Record<string, unknown>,
): Record<string, unknown> {
  const namespaces = new Set([
    ...Object.keys(base),
    ...Object.keys(overlay),
  ]);
  const result: Record<string, unknown> = {};

  for (const namespace of namespaces) {
    result[namespace] = mergeJsonValues(base[namespace], overlay[namespace]);
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
  return mergeNamespaceMaps(base, overlay) as ExtensionMap;
}

/**
 * Merge maps for round-trip preserve: target wins on known keys;
 * unknown namespaces and keys from source are retained.
 */
export function preserveExtensionMaps(
  source: ExtensionMap,
  target: ExtensionMap,
): ExtensionMap {
  return mergeNamespaceMaps(source, target) as ExtensionMap;
}

/**
 * Deep-merge two module maps; object-valued namespaces are deep-merged while
 * array-valued namespaces are replaced by the overlay. Round-trip only — no
 * matching, activation, or scoring.
 */
export function mergeModuleMaps(
  base: ModuleMap,
  overlay: ModuleMap,
): ModuleMap {
  return mergeNamespaceMaps(base, overlay) as ModuleMap;
}

/**
 * Merge module maps for round-trip preserve: target wins on known keys;
 * unknown namespaces and keys from source are retained. Round-trip only — no
 * matching, activation, or scoring.
 */
export function preserveModuleMaps(
  source: ModuleMap,
  target: ModuleMap,
): ModuleMap {
  return mergeNamespaceMaps(source, target) as ModuleMap;
}
