import type { Relation, TimelineEvent } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

function isNonEmptyTrimmedString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function readTimelineEntryId(event: TimelineEvent): string | undefined {
  const spoke = event.extensions?.spoke;
  if (spoke === null || typeof spoke !== "object" || Array.isArray(spoke)) {
    return undefined;
  }
  const entryId = (spoke as Record<string, unknown>).timeline_entry_id;
  if (!isNonEmptyTrimmedString(entryId)) {
    return undefined;
  }
  return entryId.trim();
}

/**
 * Keep TimelineEvents where timeline_scale is exactly "moment"; input order preserved.
 */
export function filterTimelineEventsByMomentScale(
  timelineEvents: TimelineEvent[],
): TimelineEvent[] {
  return timelineEvents.filter(
    (event) => event.timeline_scale === "moment",
  );
}

/**
 * Order TimelineEvents by an explicit timeline_event_id list; unknown or duplicate ids reject.
 */
export function orderTimelineEventsByIds(
  timelineEvents: TimelineEvent[],
  orderedIds: string[],
): SpokeResult<TimelineEvent[]> {
  const seenOrderedIds = new Set<string>();
  for (const id of orderedIds) {
    if (seenOrderedIds.has(id)) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        "orderedIds contains duplicate timeline_event_id values",
      );
    }
    seenOrderedIds.add(id);
  }

  const byId = new Map<string, TimelineEvent>();
  for (const event of timelineEvents) {
    byId.set(event.timeline_event_id, event);
  }

  const unknownTimelineEventIds: string[] = [];
  for (const id of orderedIds) {
    if (!byId.has(id)) {
      unknownTimelineEventIds.push(id);
    }
  }
  if (unknownTimelineEventIds.length > 0) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "orderedIds contains timeline_event_id values not present in timelineEvents",
      { unknown_timeline_event_ids: unknownTimelineEventIds },
    );
  }

  const orderedIdSet = new Set(orderedIds);
  const ordered: TimelineEvent[] = orderedIds.map((id) => byId.get(id)!);
  for (const event of timelineEvents) {
    if (!orderedIdSet.has(event.timeline_event_id)) {
      ordered.push(event);
    }
  }

  return spokeOk(ordered);
}

export type OrderTimelineEventsByPrecedesOptions = {
  relationType?: string;
};

/**
 * Topologically order linked TimelineEvents via precedes Relations on dual KE ids.
 */
export function orderTimelineEventsByPrecedes(
  timelineEvents: TimelineEvent[],
  relations: Relation[],
  options?: OrderTimelineEventsByPrecedesOptions,
): SpokeResult<TimelineEvent[]> {
  const relationType = options?.relationType ?? "precedes";

  const linkedEvents: TimelineEvent[] = [];
  const unlinkedEvents: TimelineEvent[] = [];
  const entryIdToEventId = new Map<string, string>();
  const eventIdToEntryId = new Map<string, string>();

  for (const event of timelineEvents) {
    const entryId = readTimelineEntryId(event);
    if (entryId === undefined) {
      unlinkedEvents.push(event);
      continue;
    }
    linkedEvents.push(event);
    entryIdToEventId.set(entryId, event.timeline_event_id);
    eventIdToEntryId.set(event.timeline_event_id, entryId);
  }

  const linkedEventIds = new Set(
    linkedEvents.map((event) => event.timeline_event_id),
  );
  const inDegree = new Map<string, number>();
  const adjacency = new Map<string, string[]>();

  for (const eventId of linkedEventIds) {
    inDegree.set(eventId, 0);
    adjacency.set(eventId, []);
  }

  for (const relation of relations) {
    if (relation.relation_type !== relationType) {
      continue;
    }
    if (
      !isNonEmptyTrimmedString(relation.from_id) ||
      !isNonEmptyTrimmedString(relation.to_id)
    ) {
      continue;
    }

    const fromEntryId = relation.from_id.trim();
    const toEntryId = relation.to_id.trim();
    const fromEventId = entryIdToEventId.get(fromEntryId);
    const toEventId = entryIdToEventId.get(toEntryId);
    if (fromEventId === undefined || toEventId === undefined) {
      continue;
    }
    if (fromEventId === toEventId) {
      continue;
    }

    adjacency.get(fromEventId)!.push(toEventId);
    inDegree.set(toEventId, (inDegree.get(toEventId) ?? 0) + 1);
  }

  const ready = [...linkedEventIds]
    .filter((eventId) => (inDegree.get(eventId) ?? 0) === 0)
    .sort((left, right) => left.localeCompare(right));

  const sortedLinkedIds: string[] = [];
  while (ready.length > 0) {
    const current = ready.shift()!;
    sortedLinkedIds.push(current);

    const neighbors = adjacency.get(current) ?? [];
    for (const neighbor of neighbors) {
      const nextDegree = (inDegree.get(neighbor) ?? 0) - 1;
      inDegree.set(neighbor, nextDegree);
      if (nextDegree === 0) {
        ready.push(neighbor);
      }
    }
    ready.sort((left, right) => left.localeCompare(right));
  }

  if (sortedLinkedIds.length !== linkedEventIds.size) {
    const cycleEntryIds = [...linkedEventIds]
      .filter((eventId) => (inDegree.get(eventId) ?? 0) > 0)
      .map((eventId) => eventIdToEntryId.get(eventId)!)
      .sort((left, right) => left.localeCompare(right));

    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "precedes relation graph contains a cycle among linked timeline events",
      { precedes_cycle: true, entry_ids: cycleEntryIds },
    );
  }

  const eventById = new Map(
    timelineEvents.map((event) => [event.timeline_event_id, event]),
  );
  const ordered = [
    ...sortedLinkedIds.map((eventId) => eventById.get(eventId)!),
    ...unlinkedEvents,
  ];

  return spokeOk(ordered);
}
