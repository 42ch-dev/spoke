/**
 * `@42ch/spoke-connect/remote` — opt-in RemoteAdapter module (frozen
 * contract §9: "RemoteAdapter is an opt-in module path (`remote/`)").
 *
 * Public surface: the message-oriented `Transport` interface + in-repo
 * loopback implementation, the wire-shape guards, and the drop-in async
 * `BaselinePorts` `RemoteAdapter` + `connectRemoteAdapter` dial entrypoint,
 * plus the multi-peer capability router (`connectMultiPeerRouter` /
 * `MultiPeerRouter`) composing N registered adapters behind the same
 * `BaselinePorts` surface.
 * WebSocket (or any product) transports are consumer-side and never
 * imported here.
 */

export {
  connectRemoteAdapter,
  ADOPTION_OPS,
  HISTORY_OPS,
  MEMORY_OPS,
  RemoteAdapter,
} from "./remote-adapter.js";
export type {
  RemoteAdapterOptions,
  RemoteAdapterState,
  RemoteIdentity,
  ToolHandler,
} from "./remote-adapter.js";

export { connectResponder, ConnectResponder } from "./responder.js";
export {
  knowledgeOnlyServePorts,
  withKnowledge,
} from "./responder.js";
export type {
  AdoptionAdmission,
  ConnectResponderOptions,
  ConnectResponderState,
  HistoryAdmission,
  HistoryCall,
  HistoryDedupStore,
  RemoteExtractService,
  RemoteKnowledgeFace,
  RemoteAdoptionService,
  RemoteHistoryService,
  RemoteMemoryService,
  RemoteServePorts,
} from "./responder.js";
export {
  AdoptionPortService,
  decodeAdoptionCall,
  decodeHistoryCall,
  HISTORY_OP_GET,
  HISTORY_OP_PRUNE,
  HistoryPortService,
  InMemoryHistoryDedupStore,
  KNOWLEDGE_OP_ADOPT,
} from "./serve_ports.js";

export { connectMultiPeerRouter, MultiPeerRouter } from "./multi-peer-router.js";
export type {
  MultiPeerRouterOptions,
  RoutedRemoteAdapter,
} from "./multi-peer-router.js";

export type { EnvelopeBytes, Transport } from "./transport.js";
export {
  LoopbackTransport,
  loopbackTransportPair,
} from "./transport.js";

export {
  isConnectHello,
  isConnectInvokeRequest,
  isConnectInvokeResponse,
  isConnectSession,
} from "./guards.js";
