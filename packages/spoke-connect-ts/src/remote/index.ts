/**
 * `@42ch/spoke-connect/remote` — opt-in RemoteAdapter module (frozen
 * contract §9: "RemoteAdapter is an opt-in module path (`remote/`)").
 *
 * Public surface: the message-oriented `Transport` interface + in-repo
 * loopback implementation, the wire-shape guards, and the drop-in async
 * `BaselinePorts` `RemoteAdapter` + `connectRemoteAdapter` dial entrypoint.
 * WebSocket (or any product) transports are consumer-side and never
 * imported here.
 */

export { connectRemoteAdapter, RemoteAdapter } from "./remote-adapter.js";
export type {
  RemoteAdapterOptions,
  RemoteAdapterState,
  RemoteIdentity,
} from "./remote-adapter.js";

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
