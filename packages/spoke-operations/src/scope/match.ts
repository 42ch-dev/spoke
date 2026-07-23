import type { Event, Keyblock, Scope } from "@42ch/spoke-schemas";

/**
 * Keyblock passes optional Scope refinements (AND when present).
 */
export function keyblockMatchesScope(keyblock: Keyblock, scope: Scope): boolean {
  if (scope.keyblock_ids !== undefined && !scope.keyblock_ids.includes(keyblock.keyblock_id)) {
    return false;
  }

  if (scope.block_types !== undefined && !scope.block_types.includes(keyblock.block_type)) {
    return false;
  }

  if (
    scope.source_id !== undefined &&
    keyblock.source_anchor?.source_id !== scope.source_id
  ) {
    return false;
  }

  return true;
}

/**
 * Filter Keyblocks by optional Scope refinements.
 */
export function filterKeyblocksByScope(keyblocks: Keyblock[], scope: Scope): Keyblock[] {
  return keyblocks.filter((keyblock) => keyblockMatchesScope(keyblock, scope));
}

/**
 * Event passes optional Scope refinements (AND when present).
 */
export function eventMatchesScope(event: Event, scope: Scope): boolean {
  if (scope.event_ids !== undefined && !scope.event_ids.includes(event.event_id)) {
    return false;
  }

  if (scope.timeline_scale !== undefined && event.timeline_scale !== scope.timeline_scale) {
    return false;
  }

  return true;
}

/**
 * Filter Events by optional Scope refinements.
 */
export function filterEventsByScope(events: Event[], scope: Scope): Event[] {
  return events.filter((event) => eventMatchesScope(event, scope));
}
