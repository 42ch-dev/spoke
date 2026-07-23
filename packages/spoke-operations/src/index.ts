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
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeOk,
  type SpokeReject,
  type SpokeResult,
} from "./result.js";
