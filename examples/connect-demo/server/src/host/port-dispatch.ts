/**
 * D4 port-op mapping — each `BaselinePorts` method maps to a reserved
 * `port.*` connect invoke with a snake_case opaque payload
 * (`.mstar/specs/spoke-remote-adapter.md` D4 — port-method ops over the open
 * `op` vocabulary, no schema change). Success `payload` carries the success
 * value `T`; failures travel the `ConnectInvokeResponse` error branch.
 *
 * `expected_base_revision` OCC semantics are the adapter's
 * (`spoke-operations.md` §5): `null` = create, non-null = compare-and-swap;
 * stale base → `STORED_REVISION_STALE`, impossible future base →
 * `REVISION_CONFLICT`.
 */

import type {
  Finding,
  KnowledgeEntry,
  Relation,
  Rule,
  Scope,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  spokeReject,
  type BaselinePorts,
  type SpokeResult,
} from "@42ch/spoke-operations";

/**
 * Product `op_capability_requirements` map (spec §Op dispatch gate): every
 * baseline `port.*` op requires `spoke-baseline`. The core `requiredCapability`
 * table returns `undefined` for `port.*`, so WITHOUT this map every invoke
 * would be denied `op_unsupported`.
 */
export const PORT_OP_CAPABILITY_REQUIREMENTS: Record<string, string> = {
  "port.knowledge.get": "spoke-baseline",
  "port.knowledge.put": "spoke-baseline",
  "port.relation.get": "spoke-baseline",
  "port.relation.put": "spoke-baseline",
  "port.scope.list_knowledge_entries": "spoke-baseline",
  "port.scope.list_timeline_events": "spoke-baseline",
  "port.finding.put": "spoke-baseline",
  "port.rule.list": "spoke-baseline",
  "port.host.list_peer_manifests": "spoke-baseline",
};

/** Opaque invoke `payload` shape (D4: method arguments, snake_case). */
export type PortOpPayload = Record<string, unknown>;

/**
 * Map a `port.*` op + payload to the adapter method per the D4 catalogue.
 * The dispatch gate (capability check) has already run when this is called;
 * unknown ops reject `CAPABILITY_PORT_MISSING` as a safety net for host
 * misconfiguration.
 */
export function dispatchPortOp(
  op: string,
  payload: PortOpPayload,
  adapter: BaselinePorts,
): Promise<SpokeResult<unknown>> {
  switch (op) {
    case "port.knowledge.get":
      return adapter.getKnowledgeEntry(payload.entry_id as string);
    case "port.knowledge.put":
      return adapter.putKnowledgeEntry(
        payload.entry as KnowledgeEntry,
        payload.expected_base_revision as number | null,
      );
    case "port.relation.get":
      return adapter.getRelation(payload.relation_id as string);
    case "port.relation.put":
      return adapter.putRelation(
        payload.relation as Relation,
        payload.expected_base_revision as number | null,
      );
    case "port.scope.list_knowledge_entries":
      return adapter.listKnowledgeEntries(payload.scope as Scope);
    case "port.scope.list_timeline_events":
      return adapter.listTimelineEvents(payload.scope as Scope);
    case "port.finding.put":
      return adapter.putFindings(payload.findings as Finding[]);
    case "port.rule.list":
      return adapter.listRules(payload.rule_refs as string[]);
    case "port.host.list_peer_manifests":
      return adapter.listPeerHostCapabilityManifests();
    default:
      // Unreachable when the dispatch gate denies unknown ops first; kept as
      // a safety net for host misconfiguration (mirrors loopback-host.ts).
      return Promise.resolve(
        spokeReject(
          SpokeRejectCode.CAPABILITY_PORT_MISSING,
          `unimplemented port op ${op}`,
          { op },
        ),
      );
  }
}
