export {
  mergeExtensionMaps,
  preserveExtensionMaps,
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
  validateUpsertKnowledgeEntry,
  type ValidateUpsertKnowledgeEntryContext,
} from "./upsert/validate.js";

export { validateRelateRequest } from "./relate/validate.js";

export { toErrorEnvelope, fromErrorEnvelope } from "./error/envelope.js";

export {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeOk,
  type SpokeReject,
  type SpokeResult,
} from "./result.js";
