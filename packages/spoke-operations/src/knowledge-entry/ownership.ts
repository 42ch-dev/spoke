import type { KnowledgeEntry } from "@42ch/spoke-schemas";

/**
 * Sole core disclosure value: the entry is visible only to its own holder.
 * Additional disclosure strings are Domain Profile vocabulary and stay open.
 */
const OWNER_PRIVATE_DISCLOSURE = "owner-private";

/**
 * Read the holder entry id governing a KnowledgeEntry; undefined means ownership is
 * unspecified — not a reserved world owner and not world consensus.
 */
export function getKnowledgeEntryOwner(
  entry: KnowledgeEntry,
): string | undefined {
  return entry.owner;
}

/**
 * Core disclosure predicate for a reader viewpoint. Absent disclosure is shared within
 * the caller's already pre-scoped KB; `owner-private` is visible only to a viewpoint that
 * exactly equals a non-empty owner; unknown open-vocabulary values and a malformed private
 * entry without a non-empty owner are excluded. Comparison is exact — identifiers are not
 * normalized, and no holder lookup or audience expansion occurs.
 */
export function knowledgeEntryVisibleToViewpoint(
  entry: KnowledgeEntry,
  viewpoint?: string,
): boolean {
  const disclosure = entry.disclosure;
  if (disclosure === undefined) {
    return true;
  }

  if (disclosure !== OWNER_PRIVATE_DISCLOSURE) {
    return false;
  }

  const owner = getKnowledgeEntryOwner(entry);
  if (owner === undefined || owner.length === 0 || viewpoint === undefined) {
    return false;
  }

  return owner === viewpoint;
}
