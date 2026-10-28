// Hoist deep bundle pointers into real definitions for typify.
// Extracted from run.mjs so the allocation guard is unit-testable.
export function resolveJsonPointer(doc, pointer) {
  let cur = doc;
  for (const raw of pointer.split("/").filter(Boolean)) {
    // Bundle pointers may traverse a $ref node; follow intra-document refs.
    while (cur !== null && typeof cur === "object" && typeof cur.$ref === "string" && cur.$ref.startsWith("#/")) {
      cur = resolveJsonPointer(doc, cur.$ref.slice(1));
    }
    const key = decodeURIComponent(raw).replace(/~1/g, "/").replace(/~0/g, "~");
    if (cur === null || typeof cur !== "object" || !(key in cur)) {
      throw new Error(`bundle pointer "${pointer}" does not resolve`);
    }
    cur = cur[key];
  }
  return cur;
}

/**
 * ref-parser bundle dedups repeated subschemas by pointing at deep paths such as
 * "#/definitions/X/properties/y". typify only resolves "#/definitions/<name>" refs,
 * so hoist every deep pointer into a real (content-deduplicated) definition.
 *
 * Synthesized names allocate the first absent "SharedN" — an existing definition
 * named "Shared0" must never be overwritten (regression-tested in test/).
 */
export function hoistDeepRefs(bundled) {
  const defs = (bundled.definitions ??= {});
  const byHash = new Map(Object.entries(defs).map(([name, sub]) => [JSON.stringify(sub), name]));
  const usedNames = new Set(Object.keys(defs));
  let hoistCount = 0;
  const nextName = () => {
    while (usedNames.has(`Shared${hoistCount}`)) hoistCount++;
    const name = `Shared${hoistCount}`;
    usedNames.add(name);
    return name;
  };
  const visit = (node) => {
    if (Array.isArray(node)) return node.forEach(visit);
    if (node === null || typeof node !== "object") return;
    if (
      typeof node.$ref === "string" &&
      node.$ref.startsWith("#/") &&
      !/^#\/definitions\/[^/]+$/.test(node.$ref)
    ) {
      const target = resolveJsonPointer(bundled, node.$ref.slice(1));
      const hash = JSON.stringify(target);
      let name = byHash.get(hash);
      if (name === undefined) {
        name = nextName();
        defs[name] = target;
        byHash.set(hash, name);
      }
      node.$ref = `#/definitions/${name}`;
    }
    Object.values(node).forEach(visit);
  };
  visit(bundled);
  // Hoisted definitions may themselves contain deep pointers.
  for (const [name, sub] of Object.entries(defs)) {
    if (name.startsWith("Shared")) continue;
    visit(sub);
  }
}
