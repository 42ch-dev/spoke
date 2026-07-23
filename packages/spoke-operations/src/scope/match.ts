import type { KnowledgeEntry, Scope, TimelineEvent } from "@42ch/spoke-schemas";

/**
 * KnowledgeEntry passes optional Scope refinements (AND when present).
 */
export function knowledgeEntryMatchesScope(
  knowledgeEntry: KnowledgeEntry,
  scope: Scope,
): boolean {
  if (
    scope.knowledge_entry_ids !== undefined &&
    !scope.knowledge_entry_ids.includes(knowledgeEntry.knowledge_entry_id)
  ) {
    return false;
  }

  if (scope.entry_types !== undefined && !scope.entry_types.includes(knowledgeEntry.entry_type)) {
    return false;
  }

  if (
    scope.source_id !== undefined &&
    knowledgeEntry.source_anchor?.source_id !== scope.source_id
  ) {
    return false;
  }

  return true;
}

/**
 * Filter KnowledgeEntries by optional Scope refinements.
 */
export function filterKnowledgeEntriesByScope(
  knowledgeEntries: KnowledgeEntry[],
  scope: Scope,
): KnowledgeEntry[] {
  return knowledgeEntries.filter((knowledgeEntry) =>
    knowledgeEntryMatchesScope(knowledgeEntry, scope),
  );
}

/**
 * TimelineEvent passes optional Scope refinements (AND when present).
 */
export function timelineEventMatchesScope(
  timelineEvent: TimelineEvent,
  scope: Scope,
): boolean {
  if (
    scope.timeline_event_ids !== undefined &&
    !scope.timeline_event_ids.includes(timelineEvent.timeline_event_id)
  ) {
    return false;
  }

  if (scope.timeline_scale !== undefined && timelineEvent.timeline_scale !== scope.timeline_scale) {
    return false;
  }

  return true;
}

/**
 * Filter TimelineEvents by optional Scope refinements.
 */
export function filterTimelineEventsByScope(
  timelineEvents: TimelineEvent[],
  scope: Scope,
): TimelineEvent[] {
  return timelineEvents.filter((timelineEvent) =>
    timelineEventMatchesScope(timelineEvent, scope),
  );
}
