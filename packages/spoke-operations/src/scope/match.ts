import type {
  CognitionRecord,
  CognitionSelection,
  KnowledgeEntry,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";

import { cognitionRecordMatchesFilter } from "../cognition/select.js";
import {
  validateCognitionModule,
  validateCognitionSelection,
} from "../cognition/validate.js";
import { knowledgeEntryVisibleToViewpoint } from "../knowledge-entry/ownership.js";
import {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeResult,
} from "../result.js";

/**
 * Internal authorization stage: KnowledgeEntry passes optional Scope refinements
 * (AND when present), including the core disclosure predicate applied to
 * `scope.viewpoint`.
 *
 * Cognition is a derivative selection: this boolean predicate deliberately does
 * not evaluate `scope.cognition` and is therefore not exported from the package
 * surface. Served Scope list consumers use {@link filterKnowledgeEntriesByScope},
 * which applies cognition after this stage so an unsupported requirement rejects
 * instead of silently matching every entry. Assembly builders that need the
 * pre-cognition authorized source set call this through that same internal stage.
 */
export function knowledgeEntryPassesScopeAuthorization(
  knowledgeEntry: KnowledgeEntry,
  scope: Scope,
): boolean {
  if (
    scope.entry_ids !== undefined &&
    !scope.entry_ids.includes(knowledgeEntry.entry_id)
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

  if (!knowledgeEntryVisibleToViewpoint(knowledgeEntry, scope.viewpoint)) {
    return false;
  }

  return true;
}

/**
 * Served Scope KnowledgeEntry list filter.
 *
 * Authorization/disclosure runs first, then the validated cognition selection:
 * an unsupported or unavailable requirement rejects the whole call instead of
 * silently ignoring it or matching nothing, and the absence of
 * `scope.cognition` returns the baseline authorized set unchanged.
 */
export function filterKnowledgeEntriesByScope(
  knowledgeEntries: readonly KnowledgeEntry[],
  scope: Scope,
): SpokeResult<KnowledgeEntry[]> {
  const authorizedEntries = knowledgeEntries.filter((knowledgeEntry) =>
    knowledgeEntryPassesScopeAuthorization(knowledgeEntry, scope),
  );
  return filterKnowledgeEntriesByScopeCognition(
    authorizedEntries,
    scope,
    "scope.cognition",
  );
}

function cognitionRecordsOfEntry(
  entry: KnowledgeEntry,
): SpokeResult<CognitionRecord[]> {
  const cognitionModule = entry.modules?.cognition;
  if (cognitionModule === undefined) {
    return spokeOk([]);
  }

  if (!validateCognitionModule(cognitionModule)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `Invalid cognition module on authorized entry: ${entry.entry_id}`,
      {
        reason: "invalid_cognition_record",
        entry_id: entry.entry_id,
        field: "modules.cognition",
      },
    );
  }

  // The wire-shape gate above already proved the closed module shape.
  const validatedModule = cognitionModule as { records: CognitionRecord[] };
  return spokeOk(validatedModule.records);
}

/**
 * Cognition records carried by already-authorized entries.
 *
 * Authorization/disclosure runs first: callers pass only entries that already
 * passed {@link knowledgeEntryPassesScopeAuthorization} (including the viewpoint disclosure
 * predicate), so an unauthorized holder's cognition never reaches filtering or
 * projection. Records come only from the closed `modules.cognition` carrier;
 * unknown module namespaces stay opaque. A malformed carrier rejects the whole
 * request as `invalid_cognition_record` instead of silently dropping records
 * and claiming "no cognition was recorded".
 */
export function cognitionRecordsOfAuthorizedEntries(
  authorizedEntries: readonly KnowledgeEntry[],
): SpokeResult<CognitionRecord[]> {
  const records: CognitionRecord[] = [];

  for (const entry of authorizedEntries) {
    const entryRecords = cognitionRecordsOfEntry(entry);
    if (!entryRecords.ok) {
      return entryRecords;
    }
    records.push(...entryRecords.value);
  }

  return spokeOk(records);
}

/**
 * Reject a cognition selection this KnowledgeEntry carrier cannot serve.
 *
 * The carrier holds current cognition only. `recorded_at` is a separate Memory
 * source view, so it is unavailable here rather than an ordinary empty result:
 * the whole request rejects before any source output, ranking, pagination or
 * checker/finding side effect (consumer contract §2.2). `fieldPath` is the
 * request's JSON path to the selection, so the rejection points at the actual
 * requirement.
 */
export function assertSupportedCognitionSelection(
  selection: CognitionSelection | undefined,
  fieldPath: string,
): SpokeResult<void> {
  if (selection === undefined) {
    return spokeOk(undefined);
  }

  const validated = validateCognitionSelection(selection);
  if (!validated.ok) {
    // C5 names the failing member relative to the selection; the caller's
    // `fieldPath` is this request's location for the selection root. A
    // non-identifier member already carries its own bracket, so no dot.
    const relative = validated.details?.field;
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Unsupported cognition requirement",
      {
        reason: "unsupported_cognition_requirement",
        field:
          typeof relative === "string"
            ? relative.startsWith("[")
              ? `${fieldPath}${relative}`
              : `${fieldPath}.${relative}`
            : fieldPath,
      },
    );
  }

  if (selection.view !== "current") {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Unsupported cognition requirement: recorded-at-time source is unavailable",
      { reason: "unsupported_cognition_requirement", field: `${fieldPath}.view` },
    );
  }

  return spokeOk(undefined);
}

/**
 * Applies a supported `Scope.cognition` selection to already-authorized entries.
 *
 * Order is disclosure first, cognition second: callers pass only entries that
 * passed {@link knowledgeEntryPassesScopeAuthorization}. An entry is served when
 * at least one of its carried records satisfies the one-record conjunction; a
 * selection this carrier cannot serve rejects the request instead of matching
 * nothing. Entries without carried cognition are excluded by a cognition
 * selection, and the absence of `scope.cognition` returns the baseline set
 * unchanged. The served list helper {@link filterKnowledgeEntriesByScope} runs
 * both stages, so this lower-level step is not exported from the package.
 */
export function filterKnowledgeEntriesByScopeCognition(
  authorizedEntries: readonly KnowledgeEntry[],
  scope: Scope,
  fieldPath: string,
): SpokeResult<KnowledgeEntry[]> {
  const supported = assertSupportedCognitionSelection(
    scope.cognition,
    fieldPath,
  );
  if (!supported.ok) {
    return supported;
  }

  const selection = scope.cognition;
  if (selection === undefined) {
    return spokeOk([...authorizedEntries]);
  }

  const kept: KnowledgeEntry[] = [];
  for (const entry of authorizedEntries) {
    const entryRecords = cognitionRecordsOfEntry(entry);
    if (!entryRecords.ok) {
      return entryRecords;
    }
    if (
      entryRecords.value.some(
        (record) =>
          record.view === selection.view &&
          cognitionRecordMatchesFilter(record, selection.filter),
      )
    ) {
      kept.push(entry);
    }
  }

  return spokeOk(kept);
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

  if (scope.fork_id !== undefined && timelineEvent.fork_id !== scope.fork_id) {
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
