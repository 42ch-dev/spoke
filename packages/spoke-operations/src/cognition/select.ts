import type {
  CognitionFilter,
  CognitionRecord,
  CognitionReference,
  CognitionSelection,
  MindState,
} from "@42ch/spoke-schemas";

import {
  spokeOk,
  spokeReject,
  SpokeRejectCode,
  type SpokeReject,
  type SpokeResult,
} from "../result.js";
import { validateCognitionFilter, validateCognitionSelection } from "./validate.js";

type GeneratedCognitionProjection = NonNullable<MindState["cognition"]>;

export type CognitiveView = GeneratedCognitionProjection["view"];

export type CognitionProjection = GeneratedCognitionProjection;

export type CognitionProjectionItem = CognitionProjection["items"][number];

export type CognitionProjectionItemSelection = NonNullable<
  CognitionProjectionItem["selection"]
>;

export type CognitionContext = CognitionProjectionItem["assessment_context"];

export type CognitionTarget = CognitionProjectionItem["target"];

export type CurrentViewPolicy = {
  select(input: { assessments: readonly CognitionRecord[] }): string;
};

function sameReference(a: CognitionReference, b: CognitionReference): boolean {
  return a.kind === b.kind && a.namespace === b.namespace && a.id === b.id;
}

function categoricalMatches<V>(
  predicate: { eq: V } | { absent: true } | undefined,
  value: V | undefined,
): boolean {
  if (predicate === undefined) {
    return true;
  }
  if ("eq" in predicate) {
    return value !== undefined && value === predicate.eq;
  }
  return value === undefined;
}

/**
 * One-record conjunction over the closed CognitionFilter grammar. Every
 * supplied predicate must hold on the same record; predicates never join
 * across records or targets. Reference predicates compare the complete
 * `(kind, namespace, id)` tuple — never bare ids or equal text.
 */
export function cognitionRecordMatchesFilter(
  record: CognitionRecord,
  filter: CognitionFilter,
): boolean {
  if (filter.holder !== undefined && !sameReference(record.holder, filter.holder)) {
    return false;
  }
  if (filter.target !== undefined) {
    const target = filter.target;
    if (
      target.carrier !== undefined &&
      !sameReference(record.target.carrier, target.carrier)
    ) {
      return false;
    }
    if (
      target.target_id !== undefined &&
      record.target.target_id !== target.target_id
    ) {
      return false;
    }
    if (target.kind !== undefined && record.target.kind !== target.kind) {
      return false;
    }
  }
  if (filter.assessment_context !== undefined) {
    const context = filter.assessment_context;
    if (
      context.context_id !== undefined &&
      record.assessment_context.context_id !== context.context_id
    ) {
      return false;
    }
    if (context.fork_id !== undefined) {
      const actual = record.assessment_context.fork_id;
      const matches =
        "eq" in context.fork_id
          ? actual !== undefined && actual === context.fork_id.eq
          : actual === undefined;
      if (!matches) {
        return false;
      }
    }
  }
  if (filter.category !== undefined && record.kind !== filter.category) {
    return false;
  }
  if (filter.state !== undefined) {
    if (record.kind !== "epistemic" || record.state !== filter.state) {
      return false;
    }
  }
  if (filter.order !== undefined) {
    if (record.kind !== "belief" || record.order !== filter.order) {
      return false;
    }
  }
  if (filter.q !== undefined) {
    const q = record.q.value;
    if (filter.q.min !== undefined && q < filter.q.min) {
      return false;
    }
    if (filter.q.max !== undefined && q > filter.q.max) {
      return false;
    }
  }
  const belief = record.kind === "belief" ? record : undefined;
  const hasBeliefDimension =
    filter.truth !== undefined ||
    filter.access !== undefined ||
    filter.representation !== undefined ||
    filter.content_type !== undefined ||
    filter.source !== undefined ||
    filter.belief_context !== undefined;
  if (hasBeliefDimension && belief === undefined) {
    return false;
  }
  if (!categoricalMatches(filter.truth, belief?.truth)) {
    return false;
  }
  if (!categoricalMatches(filter.access, belief?.access)) {
    return false;
  }
  if (!categoricalMatches(filter.representation, belief?.representation)) {
    return false;
  }
  if (!categoricalMatches(filter.content_type, belief?.content_type)) {
    return false;
  }
  if (!categoricalMatches(filter.source, belief?.source)) {
    return false;
  }
  if (!categoricalMatches(filter.belief_context, belief?.context)) {
    return false;
  }
  return true;
}

function unsupportedRequirement(rejection: SpokeReject): SpokeReject {
  return spokeReject(rejection.code, rejection.message, {
    ...rejection.details,
    reason: "unsupported_cognition_requirement",
  });
}

/**
 * Filters an authorized source record set by the closed filter grammar.
 * Records are returned in input order. A filter that fails the wire-shape
 * gate (unknown predicates, bare numerics, metric filters) rejects the whole
 * request as an unsupported cognition requirement — nothing is stripped.
 */
export function selectCognitionRecords(
  records: readonly CognitionRecord[],
  filter: CognitionFilter,
): SpokeResult<CognitionRecord[]> {
  const validation = validateCognitionFilter(filter);
  if (!validation.ok) {
    return unsupportedRequirement(validation);
  }
  return spokeOk(records.filter((record) => cognitionRecordMatchesFilter(record, filter)));
}

function groupKeyOf(record: CognitionRecord): string {
  return JSON.stringify([
    [record.holder.kind, record.holder.namespace, record.holder.id],
    [
      [record.target.carrier.kind, record.target.carrier.namespace, record.target.carrier.id],
      record.target.kind,
      record.target.target_id,
    ],
    [record.assessment_context.context_id, record.assessment_context.fork_id ?? null],
  ]);
}

function exactSourceContext(filter: CognitionFilter): CognitionContext | undefined {
  const context = filter.assessment_context;
  if (
    filter.holder === undefined ||
    filter.target?.carrier === undefined ||
    filter.target.target_id === undefined ||
    filter.target.kind === undefined ||
    context?.context_id === undefined ||
    context.fork_id === undefined
  ) {
    return undefined;
  }
  return {
    context_id: context.context_id,
    ...("eq" in context.fork_id ? { fork_id: context.fork_id.eq } : {}),
  };
}

/**
 * Attribution-preserving projection (§5.3). Context selection comes only from
 * the filter: matching records from every requested context are grouped by
 * compound item identity in input order, and each item keeps its own
 * `assessment_context` — there is no projection-global effective context and
 * no caller-context intersection. Every retained disagreement stays in
 * `assessments` (never averaged or latest-wins merged). A current view calls
 * the injected policy independently per compound group and writes the policy
 * identity plus all input ids into the trace; recorded-at-time projection
 * takes no callback. A `no_record` item is emitted only under the exact-source
 * rule, and only when the source has no record for that compound key before
 * optional predicates. A wildcard or partially addressed empty result is an
 * ordinary empty projection.
 */
export function projectCognitionRecords(
  records: readonly CognitionRecord[],
  selection: CognitionSelection,
  currentViewPolicy?: CurrentViewPolicy,
): SpokeResult<CognitionProjection> {
  const validation = validateCognitionSelection(selection);
  if (!validation.ok) {
    return unsupportedRequirement(validation);
  }
  const filter = selection.filter;
  const matched = records.filter(
    (record) =>
      record.view === selection.view &&
      cognitionRecordMatchesFilter(record, filter),
  );
  const groups = new Map<string, CognitionRecord[]>();
  for (const record of matched) {
    const key = groupKeyOf(record);
    const group = groups.get(key);
    if (group === undefined) {
      groups.set(key, [record]);
    } else {
      group.push(record);
    }
  }
  const items: CognitionProjectionItem[] = [];
  for (const group of groups.values()) {
    const first = group[0]!;
    const item: CognitionProjectionItem = {
      holder: first.holder,
      target: first.target,
      assessment_context: first.assessment_context,
      presence: "assessed",
      assessments: [...group],
    };
    if (selection.view === "current") {
      if (!currentViewPolicy) {
        return spokeReject(
          SpokeRejectCode.INVALID_INPUT,
          "current projection requires a current view policy",
        );
      }
      const selected = currentViewPolicy.select({ assessments: group });
      if (!group.some((record) => record.record_id === selected)) {
        return spokeReject(
          SpokeRejectCode.INVALID_INPUT,
          "current view policy must select one of the traced input record ids",
        );
      }
      item.selection = {
        policy: selection.selection_policy,
        input_record_ids: group.map((record) => record.record_id),
        selected_record_id: selected,
      };
    }
    items.push(item);
  }
  const sourceContext = exactSourceContext(filter);
  if (sourceContext !== undefined) {
    const target = filter.target!;
    const sourceHasRecord = records.some(
      (record) =>
        record.view === selection.view &&
        sameReference(record.holder, filter.holder!) &&
        sameReference(record.target.carrier, target.carrier!) &&
        record.target.target_id === target.target_id! &&
        record.target.kind === target.kind! &&
        record.assessment_context.context_id === sourceContext.context_id &&
        record.assessment_context.fork_id === sourceContext.fork_id,
    );
    if (!sourceHasRecord) {
      items.push({
        holder: filter.holder!,
        target: {
          carrier: target.carrier!,
          target_id: target.target_id!,
          kind: target.kind!,
        },
        assessment_context: sourceContext,
        presence: "no_record",
        assessments: [],
      });
    }
  }
  return spokeOk({
    view: selection.view,
    items,
    extensions: {},
  });
}
