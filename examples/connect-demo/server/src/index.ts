/**
 * Public entry of `@42ch/spoke-demo-server` — what the client e2e consumes
 * (via workspace alias to this source). `serveConnectDemo` boots the whole
 * host on one call; the remaining exports are the deterministic demo data
 * the e2e asserts against.
 */

export { serveConnectDemo } from "./transport/ws-server.js";
export type { ServeConnectDemoHandle } from "./transport/ws-server.js";
export {
  DEMO_EXTRACTION_CANARY,
  DEMO_EXTRACTION_METHOD,
  DEMO_SERVER_MANIFEST,
  MockAdapter,
  demoExtractCandidateEntryId,
} from "./adapter/mock-adapter.js";
export {
  DEMO_FOREIGN_HOLDER_ENTRY_ID,
  DEMO_FOREIGN_PRIVATE_ENTRY_ID,
  DEMO_HOLDER_ENTRY_ID,
  DEMO_OWN_PRIVATE_ENTRY_ID,
  DEMO_SEED_ENTRIES,
  DEMO_SEED_FORK_ID,
  DEMO_SEED_RELATIONS,
  DEMO_SEED_RULES,
  DEMO_SEED_TIMELINE_EVENTS,
  DEMO_SHARED_ENTRY_ID,
  DEMO_SCOPE_ID,
} from "./engine/seed-corpus.js";
export { DERIVED_WORLD_DIGEST_ENTRY_ID, MockEngine } from "./engine/mock-engine.js";
export {
  DICE_ROLL_ENTRY_ID,
  DICE_ROLL_TRIGGER_ENTRY_ID,
  DemoOrchestrator,
  ORCHESTRATION_ROLL_ARGS,
} from "./host/orchestration.js";
export type { DemoOrchestration } from "./host/orchestration.js";
export {
  DEMO_CLIENT_PEER_ID,
  DEMO_CLIENT_PUBKEY,
  DEMO_CLIENT_SEED,
  DEMO_SERVER_PEER_ID,
  DEMO_SERVER_PUBKEY,
  DEMO_SERVER_SEED,
  DEMO_STRANGER_PEER_ID,
  DEMO_STRANGER_PUBKEY,
  DEMO_STRANGER_SEED,
} from "./identities.js";
