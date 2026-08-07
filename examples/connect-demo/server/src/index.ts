/**
 * Public entry of `@42ch/spoke-demo-server` — what the client e2e consumes
 * (via workspace alias to this source). `serveConnectDemo` boots the whole
 * host on one call; the remaining exports are the deterministic demo data
 * the e2e asserts against.
 */

export { serveConnectDemo } from "./transport/ws-server.js";
export type { ServeConnectDemoHandle } from "./transport/ws-server.js";
export { DEMO_SERVER_MANIFEST, MockAdapter } from "./adapter/mock-adapter.js";
export {
  DEMO_SEED_ENTRIES,
  DEMO_SEED_RELATIONS,
  DEMO_SEED_RULES,
  DEMO_SCOPE_ID,
} from "./engine/seed-corpus.js";
export { DERIVED_WORLD_DIGEST_ENTRY_ID, MockEngine } from "./engine/mock-engine.js";
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
