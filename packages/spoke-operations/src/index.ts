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
  keyblockToAssembleEntry,
  type BuildAssemblePacketInput,
} from "./assemble/builder.js";

export { assertRevisionMatch } from "./occ/assert-revision.js";

export {
  isValidKeyblockStatusTransition,
  transitionKeyblockStatus,
} from "./keyblock/transition.js";

export {
  assertUniqueActiveKeyblock,
  type AssertUniqueActiveKeyblockInput,
} from "./keyblock/uniqueness.js";

export {
  keyblockMatchesScope,
  filterKeyblocksByScope,
  eventMatchesScope,
  filterEventsByScope,
} from "./scope/match.js";

export {
  validateUpsertKeyblock,
  type ValidateUpsertKeyblockContext,
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
