export {
  listBodyAttributes,
  filterBodyAttributesByTraitType,
  findBodyAttribute,
  type BodyAttributesInput,
} from "./body/attributes.js";

export {
  mergeExtensionMaps,
  preserveExtensionMaps,
  mergeModuleMaps,
  preserveModuleMaps,
} from "./extensions/merge.js";

export {
  isValidFindingStatusTransition,
  transitionFindingStatus,
} from "./finding/transition.js";

export {
  applyPromoteAcceptance,
  validatePromoteRequest,
} from "./promote/acceptance.js";

export {
  buildAssemblePacket,
  knowledgeEntryToAssembleEntry,
  type BuildAssemblePacketInput,
} from "./assemble/builder.js";

export { assertRevisionMatch } from "./occ/assert-revision.js";

export {
  isValidKnowledgeEntryStatusTransition,
  transitionKnowledgeEntryStatus,
} from "./knowledge-entry/transition.js";

export {
  assertUniqueActiveKnowledgeEntry,
  type AssertUniqueActiveKnowledgeEntryInput,
} from "./knowledge-entry/uniqueness.js";

export {
  knowledgeEntryMatchesScope,
  filterKnowledgeEntriesByScope,
  timelineEventMatchesScope,
  filterTimelineEventsByScope,
} from "./scope/match.js";

export {
  filterTimelineEventsByMomentScale,
  orderTimelineEventsByIds,
  orderTimelineEventsByPrecedes,
  type OrderTimelineEventsByPrecedesOptions,
} from "./timeline/sequence.js";

export {
  validateUpsertKnowledgeEntry,
  type ValidateUpsertKnowledgeEntryContext,
} from "./upsert/validate.js";

export {
  validateRelateRequest,
  type ValidateRelateRequestContext,
} from "./relate/validate.js";

export {
  validateComputableFieldMap,
  validateComputableLogEntry,
  validateProjectRequest,
  validateComputeRequest,
} from "./computable/validate.js";

export { toErrorEnvelope, fromErrorEnvelope } from "./error/envelope.js";

export {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeOk,
  type SpokeReject,
  type SpokeResult,
} from "./result.js";

export type {
  KnowledgeEntryPort,
  RelationPort,
  ScopeQueryPort,
  FindingPort,
  RuleQueryPort,
  HostManifestPort,
  ComputablePort,
  ForkTimelineQueryPort,
  BaselinePorts,
  ComputablePorts,
  ForkPorts,
  FullPorts,
  BaselineAdapter,
  ComputableAdapter,
  ForkAdapter,
  FullAdapter,
} from "./adapter/ports.js";

export {
  orchestrateUpsert,
  orchestratePromote,
  orchestrateRelate,
  orchestrateCheck,
  orchestrateAssemble,
  orchestrateProject,
  orchestrateCompute,
  orchestrateForkCheck,
  orchestrateForkAssemble,
  type CheckRunInput,
} from "./adapter/orchestrate.js";
