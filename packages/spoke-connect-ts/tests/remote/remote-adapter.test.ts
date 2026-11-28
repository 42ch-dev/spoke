import { describe, expect, it } from "vitest";

import type {
  ExtractRequest,
  CheckRequest,
  ConnectInvokeRequest,
  HostCapabilityManifest,
  KnowledgeAdoptRequest,
  KnowledgeEntry,
  Principal,
  Scope,
  UpsertRequest,
} from "@42ch/spoke-schemas";
import {
  orchestrateCheck,
  orchestrateUpsert,
  SpokeRejectCode,
  spokeOk,
  type BaselinePorts,
} from "@42ch/spoke-operations";
import { asBaselineOnly, ToyWorldAdapter } from "@42ch/spoke-fixture-toy-world";
import {
  connectRemoteAdapter,
  connectResponder,
  loopbackTransportPair,
  RemoteAdapter,
  type EnvelopeBytes,
  type Transport,
} from "@42ch/spoke-connect/remote";

import { isValidSuccessPayload } from "../../src/remote/payload.js";
import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { issueCapabilityToken } from "../../src/core/capability-token.js";
import { signHelloEd25519 } from "../../src/core/hello.js";
import { authenticateSession } from "../../src/core/envelope-auth.js";
import { generateNonce } from "../../src/core/nonce.js";
import { schemaConformantManifest } from "../fixtures/golden.js";
import { CAPABILITY_KE_EXTRACTION } from "../../src/core/dispatch.js";
import { MAX_SEQUENCE } from "../../src/core/sequence.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import { __resetAcceptedServerHellosForTest } from "../../src/remote/remote-adapter.js";
import { startLoopbackHost, type LoopbackHost } from "./loopback-host.js";

function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

function clientManifest(): HostCapabilityManifest {
  return {
    ...schemaConformantManifest(),
    host_id: "test-client",
    capabilities: ["spoke-baseline", "spoke-knowledge"],
  };
}

function freshEntry(entryId: string, canonicalName: string): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_id: entryId,
    entry_type: "character",
    canonical_name: canonicalName,
    status: "provisional",
    body: { summary: `Upserted over the loopback: ${entryId}` },
    extensions: {},
  };
}

async function dial(
  hostAdapter: ToyWorldAdapter,
  options: {
    allowlist?: readonly string[];
    clientManifest?: HostCapabilityManifest;
    invokeTimeoutMs?: number;
    hostDelay?: (request: { sequence: number }) => number;
    hostAllowlist?: readonly string[];
    hostManifest?: HostCapabilityManifest;
    hostResponseOverride?: (request: ConnectInvokeRequest) => unknown;
    attachToken?: boolean;
    clientTransport?: (clientEnd: Transport) => Transport;
  } = {},
): Promise<{
  client: RemoteAdapter;
  host: LoopbackHost;
  pair: ReturnType<typeof loopbackTransportPair>;
  peerIdHost: string;
  peerIdClient: string;
}> {
  const seedHost = seed(0xa0);
  const seedClient = seed(0x10);
  const pubkeyHost = getPublicKeyEd25519(seedHost);
  const pubkeyClient = getPublicKeyEd25519(seedClient);
  const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
  const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

  const pair = loopbackTransportPair();
  const host = await startLoopbackHost({
    transport: pair.server,
    seed: seedHost,
    clientPubkey: pubkeyClient,
    allowlist: options.hostAllowlist ?? [peerIdClient],
    adapter: hostAdapter,
    hostManifest: options.hostManifest,
    delay: options.hostDelay,
    responseOverride: options.hostResponseOverride,
  });
  const capabilityToken =
    options.attachToken === true
      ? await issueCapabilityToken(seedClient, {
          iss: peerIdClient,
          sub: peerIdClient,
          aud: peerIdHost,
          capabilities: ["spoke-baseline"],
          exp: Math.floor(Date.now() / 1000) + 3600,
        })
      : undefined;
  const client = await connectRemoteAdapter({
    transport: options.clientTransport?.(pair.client) ?? pair.client,
    localIdentity: { seed: seedClient },
    localManifest: options.clientManifest ?? clientManifest(),
    remotePubkey: pubkeyHost,
    allowlist: options.allowlist ?? [peerIdHost],
    invokeTimeoutMs: options.invokeTimeoutMs,
    capabilityToken,
  });
  return { client, host, pair, peerIdHost, peerIdClient };
}

function decodeWire(bytes: EnvelopeBytes): Record<string, unknown> {
  return decodeJsonMessage(bytes) as Record<string, unknown>;
}

function encodeWire(doc: unknown): EnvelopeBytes {
  return new TextEncoder().encode(encodeJsonMessage(doc));
}

function tamperOutboundRequests(
  clientEnd: Transport,
  mutate: (doc: Record<string, unknown>) => Record<string, unknown>,
): Transport {
  return {
    send: async (envelope) => {
      const doc = decodeWire(envelope);
      if ("op" in doc) {
        await clientEnd.send(encodeWire(mutate(doc)));
        return;
      }
      await clientEnd.send(envelope);
    },
    recv: () => clientEnd.recv(),
    close: () => clientEnd.close?.(),
  };
}

function tamperInboundResponses(
  clientEnd: Transport,
  mutate: (doc: Record<string, unknown>) => Record<string, unknown>,
): Transport {
  return {
    send: (envelope) => clientEnd.send(envelope),
    recv: async () => {
      const bytes = await clientEnd.recv();
      const doc = decodeWire(bytes);
      if ("request_id" in doc && ("payload" in doc || "error" in doc)) {
        return encodeWire(mutate(doc));
      }
      return bytes;
    },
    close: () => clientEnd.close?.(),
  };
}

describe("RemoteAdapter loopback interop", () => {
  it(
    "encapsulates connect verification and is a drop-in async BaselinePorts for orchestrateUpsert",
    async () => {
      const hostAdapter = new ToyWorldAdapter();
      const localAdapter = new ToyWorldAdapter();
      const { client, host, peerIdHost } = await dial(hostAdapter);
      try {
        const ports: BaselinePorts = client;
        expect(ports).toBe(client);
        const runtime = client as unknown as Record<string, unknown>;
        for (const hidden of [
          "allocateOutboundSequence",
          "acceptInboundSequence",
          "generateNonce",
          "signHelloEd25519",
          "verifyHelloEd25519",
          "checkResponseCorrelation",
          "isAllowlisted",
          "correlationFromRequest",
          // Envelope-auth helpers are module-internal imports (contract §9):
          // absent from the instance at runtime, exactly like the
          // session-core helpers above.
          "authenticateSession",
          "verifySessionAuth",
          "authenticateInvokeRequest",
          "verifyInvokeRequestAuth",
          "authenticateInvokeResponse",
          "verifyInvokeResponseAuth",
          "beginHandshake",
          "sendEnvelope",
          "recvEnvelope",
          "establish",
          "closeSession",
          "stateInternal",
          "session",
          "remoteManifestInternal",
          "receiveLoopRunning",
          "pending",
          "transport",
          "invokeTimeoutMs",
          "capabilityToken",
          "failAllPending",
          "runReceiveLoop",
          "receiveLoop",
          "invokeOp",
          "invokeMapped",
        ]) {
          expect(runtime[hidden], `verification helper ${hidden} must not leak`).toBeUndefined();
        }

        expect(host.stats.hellosVerified).toBe(1);
        expect(client.sessionId).toBe(host.sessionId);
        expect(client.remotePeerId).toBe(peerIdHost);
        expect(client.state).toBe("Established");
        expect(client.remoteManifest.host_id).toBe("test-host");

        const candidate = freshEntry("kb_tw_remote_cartographer", "Remote Cartographer");
        const request: UpsertRequest = { knowledge_entries: [candidate] };
        const localUpsert = await orchestrateUpsert(localAdapter, request);
        const remoteUpsert = await orchestrateUpsert(client, request);
        expect(remoteUpsert).toEqual(localUpsert);
        expect(remoteUpsert).toEqual({
          ok: true,
          value: { knowledge_entries: [candidate] },
        });
        expect(hostAdapter.store.entries.has("kb_tw_remote_cartographer")).toBe(true);

        const localConflict = await orchestrateUpsert(localAdapter, request);
        const remoteConflict = await orchestrateUpsert(client, request);
        expect(remoteConflict.ok).toBe(false);
        expect(remoteConflict).toEqual(localConflict);

        const checkRequest: CheckRequest = {
          scope: { scope_id: "toy-scope-001" },
        };
        const checker = () => ({ ok: true as const, value: [] as never[] });
        const localCheck = await orchestrateCheck(localAdapter, checkRequest, checker);
        const remoteCheck = await orchestrateCheck(client, checkRequest, checker);
        expect(remoteCheck).toEqual(localCheck);
        expect(remoteCheck.ok).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "rejects a forged Established state — verification-gating fields are ECMAScript #private (Greptile #2)",
    async () => {
      const pair = loopbackTransportPair();
      const adapter = new RemoteAdapter(
        pair.client,
        seed(0x10),
        getPublicKeyEd25519(seed(0xa0)),
        100,
      );
      const raw = adapter as unknown as Record<string, unknown>;
      raw.stateInternal = "Established";
      raw.session = {
        session_id: "forged-session",
        initiator_peer_id: "forged-initiator",
        responder_peer_id: "forged-responder",
        negotiated_capabilities: ["spoke-baseline"],
      };
      raw.remoteManifestInternal = {
        ...schemaConformantManifest(),
        host_id: "forged-host",
      };
      raw.receiveLoopRunning = true;
      raw.pending = new Map();

      expect(adapter.state).toBe("Disconnected");
      const result = await adapter.getKnowledgeEntry("kb_tw_mira");
      expect(result).toEqual({
        ok: false,
        code: SpokeRejectCode.INTERNAL_ERROR,
        message: expect.stringContaining("connect session is not established"),
        details: { kind: "session_closed" },
      });
    },
    15000,
  );

  it(
    "does not leak an unhandled rejection when Transport.close() rejects (Greptile #3)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const unhandled: unknown[] = [];
      const onUnhandled = (reason: unknown): void => {
        unhandled.push(reason);
      };
      process.on("unhandledRejection", onUnhandled);
      let host: LoopbackHost | undefined;
      try {
        const pair = loopbackTransportPair();
        const rejectingClose: Transport = {
          send: (envelope) => pair.client.send(envelope),
          recv: () => pair.client.recv(),
          close: () => Promise.reject(new Error("close boom")),
        };
        host = await startLoopbackHost({
          transport: pair.server,
          seed: seed(0xa0),
          clientPubkey: getPublicKeyEd25519(seed(0x10)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
          ],
          adapter: hostAdapter,
        });
        const client = await connectRemoteAdapter({
          transport: rejectingClose,
          localIdentity: { seed: seed(0x10) },
          localManifest: clientManifest(),
          remotePubkey: getPublicKeyEd25519(seed(0xa0)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0))),
          ],
        });
        expect(client.state).toBe("Established");

        client.close();
        await new Promise((resolve) => setTimeout(resolve, 25));
        expect(unhandled, "close() rejection must be handled").toEqual([]);
        expect(client.state).toBe("Closed");
      } finally {
        process.off("unhandledRejection", onUnhandled);
        host?.close();
      }
    },
    15000,
  );

  it(
    "does not leak an unhandled rejection when Transport.close() returns a rejecting non-native thenable (Greptile #4)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const unhandled: unknown[] = [];
      const onUnhandled = (reason: unknown): void => {
        unhandled.push(reason);
      };
      process.on("unhandledRejection", onUnhandled);
      let host: LoopbackHost | undefined;
      try {
        const pair = loopbackTransportPair();
        const rejectingThenableClose = (): unknown => {
          const inner = Promise.reject(new Error("close boom"));
          return {
            then(
              onFulfilled?: (value: never) => unknown,
              onRejected?: (reason: unknown) => unknown,
            ) {
              inner.then(onFulfilled, onRejected);
            },
          };
        };
        const thenableClose: Transport = {
          send: (envelope) => pair.client.send(envelope),
          recv: () => pair.client.recv(),
          close: rejectingThenableClose as () => void,
        };
        host = await startLoopbackHost({
          transport: pair.server,
          seed: seed(0xa0),
          clientPubkey: getPublicKeyEd25519(seed(0x10)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
          ],
          adapter: hostAdapter,
        });
        const client = await connectRemoteAdapter({
          transport: thenableClose,
          localIdentity: { seed: seed(0x10) },
          localManifest: clientManifest(),
          remotePubkey: getPublicKeyEd25519(seed(0xa0)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0))),
          ],
        });
        expect(client.state).toBe("Established");

        client.close();
        await new Promise((resolve) => setTimeout(resolve, 25));
        expect(
          unhandled,
          "thenable close() rejection must be handled",
        ).toEqual([]);
        expect(client.state).toBe("Closed");
      } finally {
        process.off("unhandledRejection", onUnhandled);
        host?.close();
      }
    },
    15000,
  );

  it(
    "does not let a synchronous Transport.close() throw escape (Greptile #5)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      let host: LoopbackHost | undefined;
      try {
        const pair = loopbackTransportPair();
        const throwingClose: Transport = {
          send: (envelope) => pair.client.send(envelope),
          recv: () => pair.client.recv(),
          close: () => {
            throw new Error("close boom");
          },
        };
        host = await startLoopbackHost({
          transport: pair.server,
          seed: seed(0xa0),
          clientPubkey: getPublicKeyEd25519(seed(0x10)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
          ],
          adapter: hostAdapter,
        });
        const client = await connectRemoteAdapter({
          transport: throwingClose,
          localIdentity: { seed: seed(0x10) },
          localManifest: clientManifest(),
          remotePubkey: getPublicKeyEd25519(seed(0xa0)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0))),
          ],
        });
        expect(client.state).toBe("Established");

        expect(() => client.close()).not.toThrow();
        expect(client.state).toBe("Closed");
      } finally {
        host?.close();
      }
    },
    15000,
  );

  it(
    "rejects a replayed server hello — accepted (peer_id, nonce) pairs are single-use (Greptile #1)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const seedHost = seed(0xa0);
      const seedClient = seed(0x10);
      const pubkeyHost = getPublicKeyEd25519(seedHost);
      const pubkeyClient = getPublicKeyEd25519(seedClient);
      const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
      const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

      const pair1 = loopbackTransportPair();
      const captured: EnvelopeBytes[] = [];
      const recording: Transport = {
        send: (envelope) => pair1.client.send(envelope),
        recv: async () => {
          const bytes = await pair1.client.recv();
          captured.push(bytes);
          return bytes;
        },
        close: () => pair1.client.close(),
      };
      const host = await startLoopbackHost({
        transport: pair1.server,
        seed: seedHost,
        clientPubkey: pubkeyClient,
        allowlist: [peerIdClient],
        adapter: hostAdapter,
      });
      const client = await connectRemoteAdapter({
        transport: recording,
        localIdentity: { seed: seedClient },
        localManifest: clientManifest(),
        remotePubkey: pubkeyHost,
        allowlist: [peerIdHost],
      });
      expect(client.state).toBe("Established");
      client.close();
      host.close();
      expect(captured.length).toBeGreaterThanOrEqual(2);
      const [replayHello, replaySession] = captured;

      let replayIndex = 0;
      const replayTransport: Transport = {
        send: async () => {},
        recv: async () => {
          if (replayIndex === 0) {
            replayIndex += 1;
            return replayHello;
          }
          if (replayIndex === 1) {
            replayIndex += 1;
            return replaySession;
          }
          throw new Error("scripted replay transport closed");
        },
        close: () => {},
      };
      await expect(
        connectRemoteAdapter({
          transport: replayTransport,
          localIdentity: { seed: seedClient },
          localManifest: clientManifest(),
          remotePubkey: pubkeyHost,
          allowlist: [peerIdHost],
        }),
      ).rejects.toThrow(/replay|dial binding/);
    },
    15000,
  );

  it(
    "rejects a replayed server hello across a simulated restart — responder peer_nonce no longer matches (Greptile P1 dial binding)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const seedHost = seed(0xa0);
      const seedClient = seed(0x10);
      const pubkeyHost = getPublicKeyEd25519(seedHost);
      const pubkeyClient = getPublicKeyEd25519(seedClient);
      const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
      const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

      const pair1 = loopbackTransportPair();
      const captured: EnvelopeBytes[] = [];
      const recording: Transport = {
        send: (envelope) => pair1.client.send(envelope),
        recv: async () => {
          const bytes = await pair1.client.recv();
          captured.push(bytes);
          return bytes;
        },
        close: () => pair1.client.close(),
      };
      const host = await startLoopbackHost({
        transport: pair1.server,
        seed: seedHost,
        clientPubkey: pubkeyClient,
        allowlist: [peerIdClient],
        adapter: hostAdapter,
      });
      const client = await connectRemoteAdapter({
        transport: recording,
        localIdentity: { seed: seedClient },
        localManifest: clientManifest(),
        remotePubkey: pubkeyHost,
        allowlist: [peerIdHost],
      });
      expect(client.state).toBe("Established");
      client.close();
      host.close();
      expect(captured.length).toBeGreaterThanOrEqual(2);
      const [replayHello] = captured;

      __resetAcceptedServerHellosForTest();

      let replayIndex = 0;
      const replayTransport: Transport = {
        send: async () => {},
        recv: async () => {
          if (replayIndex === 0) {
            replayIndex += 1;
            return replayHello;
          }
          throw new Error("scripted replay transport closed");
        },
        close: () => {},
      };
      await expect(
        connectRemoteAdapter({
          transport: replayTransport,
          localIdentity: { seed: seedClient },
          localManifest: clientManifest(),
          remotePubkey: pubkeyHost,
          allowlist: [peerIdHost],
        }),
      ).rejects.toThrow(/dial binding/);
    },
    15000,
  );

  it(
    "fails the dial fail-closed when the responder hello omits peer_nonce (mixed-version downgrade)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const seedHost = seed(0xa0);
      const seedClient = seed(0x10);
      const pubkeyHost = getPublicKeyEd25519(seedHost);
      const pubkeyClient = getPublicKeyEd25519(seedClient);
      const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
      const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

      const legacyHello = await signHelloEd25519(
        seedHost,
        "legacy-host-nonce-1234567",
        schemaConformantManifest(),
      );
      const legacyBytes = new TextEncoder().encode(encodeJsonMessage(legacyHello));

      const legacyTransport: Transport = {
        send: async () => {},
        recv: async () => legacyBytes,
        close: () => {},
      };
      await expect(
        connectRemoteAdapter({
          transport: legacyTransport,
          localIdentity: { seed: seedClient },
          localManifest: clientManifest(),
          remotePubkey: pubkeyHost,
          allowlist: [peerIdHost],
        }),
      ).rejects.toThrow(/dial binding/);
    },
    15000,
  );

  it(
    "fails the dial when the session snapshot opened_at is not a strict timestamp (qc1 F-005)",
    async () => {
      const seedHost = seed(0xa0);
      const seedClient = seed(0x10);
      const pubkeyHost = getPublicKeyEd25519(seedHost);
      const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
      const peerIdClient = derivePeerIdFromEd25519Pubkey(
        getPublicKeyEd25519(seedClient),
      );
      const textEncoder = new TextEncoder();

      let capturedInitiatorNonce: string | undefined;
      let received = 0;
      const openedAtCases: unknown[] = [
        "2026-07-23T09:00:00",
        ["2026-07-23T09:00:00Z"],
      ];
      for (const [index, openedAt] of openedAtCases.entries()) {
        capturedInitiatorNonce = undefined;
        received = 0;
        const scriptedTransport: Transport = {
          send: async (bytes) => {
            const doc: unknown = decodeJsonMessage(bytes);
            if (
              typeof doc === "object" &&
              doc !== null &&
              "nonce" in doc &&
              typeof doc.nonce === "string"
            ) {
              capturedInitiatorNonce = doc.nonce;
            }
          },
          recv: async () => {
            received += 1;
            if (received === 1) {
              if (capturedInitiatorNonce === undefined) {
                throw new Error("initiator hello was not observed");
              }
              const hello = await signHelloEd25519(
                seedHost,
                generateNonce(),
                schemaConformantManifest(),
                capturedInitiatorNonce,
              );
              return textEncoder.encode(encodeJsonMessage(hello));
            }
            if (received === 2) {
              const session = await authenticateSession(seedHost, {
                session_id: `sess-invalid-opened-at-${index}`,
                initiator_peer_id: peerIdClient,
                responder_peer_id: peerIdHost,
                opened_at: openedAt as string,
                negotiated_capabilities: ["spoke-baseline"],
                initial_sequence: 0,
              });
              return textEncoder.encode(encodeJsonMessage(session));
            }
            throw new Error("scripted transport closed");
          },
          close: () => {},
        };

        await expect(
          connectRemoteAdapter({
            transport: scriptedTransport,
            localIdentity: { seed: seedClient },
            localManifest: clientManifest(),
            remotePubkey: pubkeyHost,
            allowlist: [peerIdHost],
          }),
        ).rejects.toThrow(/opened_at/);
      }
    },
    15000,
  );

  it(
    "propagates protocol_version_mismatch through the dial when the responder hello version mismatches",
    async () => {
      const seedHost = seed(0xa0);
      const seedClient = seed(0x10);
      const pubkeyHost = getPublicKeyEd25519(seedHost);
      const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);

      const hello = await signHelloEd25519(
        seedHost,
        "mixed-version-host-nonce-1234",
        schemaConformantManifest(),
        "initiator-nonce-12345678",
      );
      const mismatched = { ...hello, protocol_version: 2 };
      const bytes = new TextEncoder().encode(encodeJsonMessage(mismatched));

      const mixedVersionTransport: Transport = {
        send: async () => {},
        recv: async () => bytes,
        close: () => {},
      };
      await expect(
        connectRemoteAdapter({
          transport: mixedVersionTransport,
          localIdentity: { seed: seedClient },
          localManifest: clientManifest(),
          remotePubkey: pubkeyHost,
          allowlist: [peerIdHost],
        }),
      ).rejects.toThrowError(
        expect.objectContaining({
          name: "CoreError",
          code: "protocol_version_mismatch",
        }),
      );
    },
    15000,
  );

  it(
    "demuxes concurrent invokes by request_id with out-of-order responses",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        hostDelay: (req) => (req.sequence === 0 ? 30 : 0),
      });
      try {
        const [first, second] = await Promise.all([
          client.getKnowledgeEntry("kb_tw_mira"),
          client.getKnowledgeEntry("kb_tw_harbor"),
        ]);
        expect(first.ok && first.value.entry_id).toBe("kb_tw_mira");
        expect(second.ok && second.value.entry_id).toBe("kb_tw_harbor");
        expect(host.stats.responseOrder).toEqual([1, 0]);
        expect(host.stats.invokesDispatched).toBe(2);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "maps an invoke timeout to INTERNAL_ERROR kind timeout without closing the session",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      let delayMs = 200;
      const { client, host } = await dial(hostAdapter, {
        invokeTimeoutMs: 100,
        hostDelay: () => delayMs,
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        expect(client.state).toBe("Established");
        delayMs = 0;
        const retry = await client.getKnowledgeEntry("kb_tw_mira");
        expect(retry.ok).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "closes the session when a timed-out queued invoke's send is skipped (never transmitted, no silent poisoning)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const sentSequences: number[] = [];
      const receivedSequences: number[] = [];
      let firstSendDelayed = false;
      const { client, host } = await dial(hostAdapter, {
        invokeTimeoutMs: 100,
        clientTransport: (clientEnd) => ({
          send: async (envelope) => {
            const doc = decodeWire(envelope);
            if ("op" in doc) {
              if (!firstSendDelayed) {
                firstSendDelayed = true;
                await new Promise((resolve) => setTimeout(resolve, 200));
              }
              sentSequences.push(doc.sequence as number);
            }
            await clientEnd.send(envelope);
          },
          recv: async () => {
            const bytes = await clientEnd.recv();
            const doc = decodeWire(bytes);
            if ("request_id" in doc && ("payload" in doc || "error" in doc)) {
              receivedSequences.push(doc.sequence as number);
            }
            return bytes;
          },
          close: () => clientEnd.close?.(),
        }),
      });
      try {
        const [first, second] = await Promise.all([
          client.getKnowledgeEntry("kb_tw_mira"),
          client.getKnowledgeEntry("kb_tw_harbor"),
        ]);
        expect(first).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        expect(second).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        const closeDeadline = Date.now() + 5000;
        while (
          client.state !== "Closed" ||
          host.stats.responseOrder.length === 0
        ) {
          if (Date.now() >= closeDeadline) {
            throw new Error(
              "adapter never closed / host never finished processing after the skipped send",
            );
          }
          await new Promise((resolve) => setTimeout(resolve, 10));
        }
        expect(sentSequences).toEqual([0]);
        expect(receivedSequences).toEqual([]);
        expect(client.state).toBe("Closed");
        expect(host.stats.invokesDispatched).toBe(1);
        const after = await client.getKnowledgeEntry("kb_tw_mira");
        expect(after).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("connect session is not established"),
          details: { kind: "session_closed" },
        });
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "fails pending invokes with INTERNAL_ERROR kind session_closed when the transport closes mid-flight",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        hostDelay: () => 100,
      });
      try {
        const pending = client.getKnowledgeEntry("kb_tw_mira");
        host.close();
        const result = await pending;
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("connect session closed"),
          details: { kind: "session_closed" },
        });
        expect(client.state).toBe("Closed");
        const after = await client.getKnowledgeEntry("kb_tw_mira");
        expect(after.ok).toBe(false);
        expect(after).toMatchObject({ code: SpokeRejectCode.INTERNAL_ERROR });
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "maps host dispatch denials to CAPABILITY_PORT_MISSING",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const noBaseline: HostCapabilityManifest = {
        ...schemaConformantManifest(),
        host_id: "test-client",
        capabilities: ["l2-computable"],
      };
      const { client, host } = await dial(hostAdapter, {
        clientManifest: noBaseline,
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("not authorized"),
          details: { wire_code: "op_unsupported" },
        });
        expect(host.stats.dispatchDenials).toBe(1);
        expect(host.stats.invokesDispatched).toBe(0);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "HostManifestPort: get returns the remote hello host from cache; listPeer proxies",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter);
      try {
        const self = await client.getHostCapabilityManifest();
        expect(self.ok && self.value.host_id).toBe("test-host");
        expect(host.stats.invokesDispatched).toBe(0);

        const peers = await client.listPeerHostCapabilityManifests();
        expect(peers.ok && peers.value.map((m) => m.host_id)).toEqual([
          "host_tw_peer",
        ]);
        expect(host.stats.invokesDispatched).toBe(1);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "attaches a configured capability token as auth on outbound invokes",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, { attachToken: true });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result.ok).toBe(true);
        expect(host.stats.authSeen).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "fails dial fail-closed when the remote peer is not on the allowlist",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const pair = loopbackTransportPair();
      const foreignPeer = derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x70)));
      const host = await startLoopbackHost({
        transport: pair.server,
        seed: seed(0xa0),
        clientPubkey: getPublicKeyEd25519(seed(0x10)),
        allowlist: [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10)))],
        adapter: hostAdapter,
      });
      try {
        await expect(
          connectRemoteAdapter({
            transport: pair.client,
            localIdentity: { seed: seed(0x10) },
            localManifest: clientManifest(),
            remotePubkey: getPublicKeyEd25519(seed(0xa0)),
            allowlist: [foreignPeer],
          }),
        ).rejects.toThrow(/not on the allowlist/);
      } finally {
        host.close();
      }
    },
    15000,
  );

  it(
    "fails dial when the host rejects the client hello (host-side allowlist)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const pubkeyClient = getPublicKeyEd25519(seed(0x10));
      const clientPeerId = derivePeerIdFromEd25519Pubkey(pubkeyClient);
      const otherPeerId = derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x20)));
      const pair = loopbackTransportPair();
      const host = await startLoopbackHost({
        transport: pair.server,
        seed: seed(0xa0),
        clientPubkey: pubkeyClient,
        allowlist: [otherPeerId],
        adapter: hostAdapter,
      }).catch(() => null);
      try {
        await expect(
          connectRemoteAdapter({
            transport: pair.client,
            localIdentity: { seed: seed(0x10) },
            localManifest: clientManifest(),
            remotePubkey: getPublicKeyEd25519(seed(0xa0)),
            allowlist: [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0)))],
            invokeTimeoutMs: 2000,
          }),
        ).rejects.toThrow();
      } finally {
        host?.close();
      }
    },
    15000,
  );

  it(
    "rejects a malformed success payload with INTERNAL_ERROR kind transport (Rust §8.2 parity)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        hostResponseOverride: (request) => ({
          session_id: request.session_id,
          sequence: request.sequence,
          request_id: request.request_id,
          payload: { garbage: true },
          extensions: {},
        }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("payload decode failed"),
          details: { kind: "transport" },
        });
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "rejects an ordinary knowledge/relation get response outside the shared scalar domains (qc1 F-003)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const entry = {
        schema_version: 1,
        entry_id: "kb_tw_mira",
        entry_type: "character",
        canonical_name: "Mira Vale",
        status: "confirmed",
        body: {},
        extensions: {},
      };
      const relation = {
        schema_version: 1,
        relation_id: "rel_boundary",
        relation_type: "related_to",
        from_id: "kb_from",
        to_id: "kb_to",
        extensions: {},
      };
      const malformed: Record<string, unknown> = {
        kb_schema_version_zero: { ...entry, schema_version: 0 },
        kb_revision_negative: { ...entry, revision: -1 },
        kb_revision_over: { ...entry, revision: 1e20 },
        kb_created_at_naive: {
          ...entry,
          created_at: "2026-07-23T09:00:00",
        },
        rel_revision_negative: { ...relation, revision: -1 },
        rel_updated_at_naive: {
          ...relation,
          updated_at: "2026-07-23 09:00:00Z",
        },
      };
      const { client, host } = await dial(hostAdapter, {
        hostResponseOverride: (request) => {
          const requestPayload = request.payload as {
            entry_id?: string;
            relation_id?: string;
          };
          const requestedId =
            requestPayload.entry_id ?? requestPayload.relation_id ?? "";
          return {
            session_id: request.session_id,
            sequence: request.sequence,
            request_id: request.request_id,
            payload: malformed[requestedId] ?? entry,
            extensions: {},
          };
        },
      });
      try {
        for (const requestedId of Object.keys(malformed)) {
          const isRelation = requestedId.startsWith("rel_");
          const result = isRelation
            ? await client.getRelation(requestedId)
            : await client.getKnowledgeEntry(requestedId);
          expect(result, requestedId).toEqual({
            ok: false,
            code: SpokeRejectCode.INTERNAL_ERROR,
            message: expect.stringContaining("payload decode failed"),
            details: { kind: "transport" },
          });
        }
        const valid = await client.getKnowledgeEntry("kb_tw_mira");
        expect(valid.ok).toBe(true);
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it("validates schema-owned wire scalars on the declared response shapes (qc1 F-003 / R1-001)", () => {
    const receipt = {
      schema_version: 1,
      receipt_id: "rcpt_1",
      op: "memory.capture",
      space_id: "space_1",
      requester_principal_id: "prn_1",
      payload_digest: "sha256:00",
      status: "committed",
      created_at: "2026-07-23T09:00:00Z",
      extensions: {},
    };
    expect(
      isValidSuccessPayload("memory.receipts.get", {
        receipt: { ...receipt, completed_at: "2026-07-23T09:00:00" },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("memory.receipts.get", {
        receipt: { ...receipt, completed_at: "2026-07-23T09:00:00+02:00" },
      }),
    ).toBe(true);

    const grant = {
      schema_version: 1,
      grant_id: "grant_1",
      space_id: "space_1",
      principal_id: "prn_1",
      capabilities: [],
      context_ids: [],
      created_at: "2026-07-23T09:00:00Z",
      revision: 1,
      extensions: {},
    };
    expect(
      isValidSuccessPayload("memory.grants.list", {
        grants: [{ ...grant, expires_at: "2026-07-23T09:00:00" }],
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("memory.grants.list", {
        grants: [{ ...grant, expires_at: "2026-07-23T09:00:00Z" }],
      }),
    ).toBe(true);

    const candidate = {
      schema_version: 1,
      entry_id: "kb_candidate",
      entry_type: "character",
      canonical_name: "Candidate",
      status: "provisional",
      body: {},
      extensions: {},
    };
    expect(
      isValidSuccessPayload("extract", {
        candidates: [{ ...candidate, revision: 2 ** 53 + 2 }],
        run: { run_id: "run_1" },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("extract", {
        candidates: [{ ...candidate, created_at: "2026-07-23T09:00:00" }],
        run: { run_id: "run_1" },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("extract", {
        candidates: [
          { ...candidate, created_at: "2026-07-23T09:00:00Z", revision: 2 ** 53 },
        ],
        run: { run_id: "run_1" },
      }),
    ).toBe(true);

    expect(
      isValidSuccessPayload("port.knowledge.put", {
        ...candidate,
        adoption_anchor: {
          schema_version: 1,
          source_scope_id: "scope_src",
          source_entry_id: "kb_src",
          source_revision: 2 ** 53 + 2,
          extensions: {},
        },
      }),
    ).toBe(false);

    const finding = {
      schema_version: 1,
      finding_id: "fnd_1",
      severity: "minor",
      status: "open",
      title: "Title",
      description: "Description",
      extensions: {},
    };
    expect(
      isValidSuccessPayload("port.finding.put", [
        { ...finding, text_position: { revision: "draft", schema_version: "x" } },
      ]),
    ).toBe(true);
    expect(
      isValidSuccessPayload("port.finding.put", [
        { ...finding, extensions: { ns: { revision: "draft" } } },
      ]),
    ).toBe(true);

    const chunk = {
      export_id: "exp_1",
      chunk_ordinal: 1,
      chunk_count: 2,
      bundle_digest: "a".repeat(64),
      chunk_digest: "b".repeat(64),
      bytes: 2,
    };
    expect(isValidSuccessPayload("memory.exports.get", { chunk })).toBe(true);
    expect(
      isValidSuccessPayload("memory.exports.get", {
        chunk: {
          ...chunk,
          chunk_ordinal: 0,
          header: { schema_version: 2 ** 53 },
        },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("memory.exports.get", {
        chunk: { ...chunk, chunk_ordinal: 0, header: { schema_version: 1 } },
      }),
    ).toBe(true);
    expect(
      isValidSuccessPayload("memory.exports.get", {
        chunk: { ...chunk, records: [{ revision: 2 ** 53 + 2 }] },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("memory.exports.get", {
        chunk: { ...chunk, records: [{ revision: 2 ** 53 }] },
      }),
    ).toBe(true);

    const record = {
      schema_version: 1,
      occurred_at: "2026-07-23T09:00:00Z",
      created_at: "2026-07-23T09:00:00Z",
      updated_at: "2026-07-23T09:00:00Z",
      revision: 1,
      provenance: { captured_at: "2026-07-23T09:00:00Z" },
    };
    expect(isValidSuccessPayload("memory.records.get", { record })).toBe(true);
    expect(
      isValidSuccessPayload("memory.records.get", {
        record: { ...record, revision: 0 },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("memory.records.get", {
        record: {
          ...record,
          import_ref: {
            import_id: "imp_1",
            source_record_id: "rec_src",
            source_revision: 2 ** 53 + 2,
          },
        },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("memory.records.get", {
        record: {
          ...record,
          import_ref: {
            import_id: "imp_1",
            source_record_id: "rec_src",
            source_revision: 2 ** 53,
          },
        },
      }),
    ).toBe(true);
    expect(
      isValidSuccessPayload("memory.records.get", {
        record: {
          ...record,
          tombstone: { deleted_at: "2026-07-23T09:00:00", deleted_by: "prn_1" },
        },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("memory.records.get", {
        record: {
          ...record,
          tombstone: { deleted_at: "2026-07-23T09:00:00Z", deleted_by: "prn_1" },
        },
      }),
    ).toBe(true);
  });

  it(
    "refuses a mixed knowledge.adopt payload (success members + error) as INTERNAL_ERROR kind transport (qc1 F-032)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const adoptionCapabilities: [string, ...string[]] = [
        "spoke-baseline",
        "spoke-knowledge",
        "spoke-knowledge-adoption",
      ];
      const { client, host } = await dial(hostAdapter, {
        clientManifest: {
          ...clientManifest(),
          capabilities: adoptionCapabilities,
        },
        hostManifest: {
          ...schemaConformantManifest(),
          capabilities: adoptionCapabilities,
        },
        hostResponseOverride: (request) => ({
          session_id: request.session_id,
          sequence: request.sequence,
          request_id: request.request_id,
          payload: {
            outcome: "adopted",
            entry: { entry_id: "ke_mixed" },
            error: {
              code: "REVISION_CONFLICT",
              message: "conflict",
              extensions: {},
            },
          },
          extensions: {},
        }),
      });
      try {
        const principal: Principal = {
          principal_id: "prn_client",
          extensions: {},
        };
        const scope: Scope = { scope_id: "spc_dst", extensions: {} };
        const request: KnowledgeAdoptRequest = {
          source_scope_id: "spc_src",
          source_entry_id: "ke_src",
          source_revision: 1,
          destination_scope_id: "spc_dst",
          idempotency_key: "k-mixed",
          extensions: {},
        };
        const result = await client.adoptKnowledgeEntry(
          principal,
          scope,
          request,
        );
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("payload decode failed"),
          details: { kind: "transport" },
        });
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "validates the knowledge.adopt success branch against the published schema (entry completeness, outcome vocabulary, closed members, extensions) (qc1 F-033)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const adoptionCapabilities: [string, ...string[]] = [
        "spoke-baseline",
        "spoke-knowledge",
        "spoke-knowledge-adoption",
      ];
      const dialWithPayload = (payload: unknown) =>
        dial(hostAdapter, {
          clientManifest: {
            ...clientManifest(),
            capabilities: adoptionCapabilities,
          },
          hostManifest: {
            ...schemaConformantManifest(),
            capabilities: adoptionCapabilities,
          },
          hostResponseOverride: (request) => ({
            session_id: request.session_id,
            sequence: request.sequence,
            request_id: request.request_id,
            payload,
            extensions: {},
          }),
        });
      const principal: Principal = {
        principal_id: "prn_client",
        extensions: {},
      };
      const scope: Scope = { scope_id: "spc_dst", extensions: {} };
      const request: KnowledgeAdoptRequest = {
        source_scope_id: "spc_src",
        source_entry_id: "ke_src",
        source_revision: 1,
        destination_scope_id: "spc_dst",
        idempotency_key: "k-f033",
        extensions: {},
      };
      const anchoredEntry: KnowledgeEntry = {
        ...freshEntry("ke_dst", "dst"),
        adoption_anchor: {
          schema_version: 1,
          source_scope_id: "spc_src",
          source_entry_id: "ke_src",
          source_revision: 1,
          extensions: {},
        },
      };

      // (a) entry without `adoption_anchor` → INTERNAL_ERROR transport.
      const unanchored = await dialWithPayload({
        outcome: "adopted",
        entry: freshEntry("ke_dst", "dst"),
      });
      try {
        const result = await unanchored.client.adoptKnowledgeEntry(
          principal,
          scope,
          request,
        );
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("payload decode failed"),
          details: { kind: "transport" },
        });
      } finally {
        unanchored.client.close();
        unanchored.host.close();
      }

      // (b) a complete anchored entry serves verbatim.
      const valid = await dialWithPayload({
        outcome: "adopted",
        entry: anchoredEntry,
      });
      try {
        const result = await valid.client.adoptKnowledgeEntry(
          principal,
          scope,
          request,
        );
        expect(result.ok).toBe(true);
        if (result.ok) {
          expect(result.value).toEqual({
            outcome: "adopted",
            entry: anchoredEntry,
          });
        }
      } finally {
        valid.client.close();
        valid.host.close();
      }

      // (c) schema-level success rules — closed outcome vocabulary, closed
      // top-level member set, ExtensionMap extensions — each refused
      // (qc1 round-15; the typed Rust decode enforces the same rules).
      const refuse = async (payload: unknown, label: string) => {
        const dialed = await dialWithPayload(payload);
        try {
          const result = await dialed.client.adoptKnowledgeEntry(
            principal,
            scope,
            request,
          );
          expect(result, label).toEqual({
            ok: false,
            code: SpokeRejectCode.INTERNAL_ERROR,
            message: expect.stringContaining("payload decode failed"),
            details: { kind: "transport" },
          });
        } finally {
          dialed.client.close();
          dialed.host.close();
        }
      };
      await refuse(
        { outcome: "unexpected", entry: anchoredEntry },
        "outcome outside the closed vocabulary",
      );
      await refuse(
        { outcome: "adopted", entry: anchoredEntry, surprise: true },
        "unknown top-level member",
      );
      await refuse(
        { outcome: "adopted", entry: anchoredEntry, extensions: [] },
        "extensions is not an ExtensionMap",
      );

      // (d) a valid `extensions` member serves verbatim.
      const extended = await dialWithPayload({
        outcome: "already_present",
        entry: anchoredEntry,
        extensions: { product: { v: 1 } },
      });
      try {
        const result = await extended.client.adoptKnowledgeEntry(
          principal,
          scope,
          request,
        );
        expect(result.ok).toBe(true);
        if (result.ok) {
          expect(result.value).toEqual({
            outcome: "already_present",
            entry: anchoredEntry,
            extensions: { product: { v: 1 } },
          });
        }
      } finally {
        extended.client.close();
        extended.host.close();
      }
    },
    15000,
  );

  it(
    "maps a correlation mismatch to INTERNAL_ERROR kind correlation_mismatch without closing the session",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      let mangled = true;
      const { client, host } = await dial(hostAdapter, {
        hostResponseOverride: (request) => {
          if (!mangled) {
            return undefined;
          }
          mangled = false;
          return {
            session_id: request.session_id,
            sequence: request.sequence + 1,
            request_id: request.request_id,
            payload: {},
            extensions: {},
          };
        },
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: "response echo fields do not match the request",
          details: { kind: "correlation_mismatch" },
        });
        expect(client.state).toBe("Established");
        const retry = await client.getKnowledgeEntry("kb_tw_mira");
        expect(retry.ok).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "maps outbound sequence exhaustion to INTERNAL_ERROR kind sequence_exhausted and closes the session",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter);
      try {
        client.setOutboundNextForTest(MAX_SEQUENCE + 1);

        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("sequence"),
          details: { kind: "sequence_exhausted" },
        });
        expect(client.state).toBe("Closed");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "round-trips an authenticated invoke (host verifies client request signatures; client verifies host response signatures)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter);
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result.ok).toBe(true);
        expect(host.stats.authRejections).toBe(0);
        expect(host.stats.invokesDispatched).toBe(1);
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "rejects a wire-tampered invoke response with envelope_auth_invalid (fail-closed, session stays usable)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      let tampered = false;
      const { client, host } = await dial(hostAdapter, {
        clientTransport: (clientEnd) =>
          tamperInboundResponses(clientEnd, (doc) => {
            if (!tampered && "payload" in doc) {
              tampered = true;
              (doc.payload as Record<string, unknown>).tampered = true;
            }
            return doc;
          }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature does not verify"),
          details: { kind: "envelope_auth_invalid" },
        });
        expect(client.state).toBe("Established");
        const retry = await client.getKnowledgeEntry("kb_tw_mira");
        expect(retry.ok).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "rejects an invoke response with a stripped signature (envelope_auth_missing)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        clientTransport: (clientEnd) =>
          tamperInboundResponses(clientEnd, (doc) => {
            delete doc.signature;
            return doc;
          }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature"),
          details: { kind: "envelope_auth_missing" },
        });
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "host rejects a wire-tampered invoke request with auth_failed envelope_auth_invalid",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        clientTransport: (clientEnd) =>
          tamperOutboundRequests(clientEnd, (doc) => {
            (doc.payload as Record<string, unknown>).tampered = true;
            return doc;
          }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature does not verify"),
          details: { kind: "envelope_auth_invalid" },
        });
        expect(host.stats.authRejections).toBe(1);
        expect(host.stats.invokesDispatched).toBe(0);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "host rejects an invoke request with a stripped signature (auth_failed envelope_auth_missing)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        // Wire-level injector: strip the request signature — the host must
        // treat the unauthenticated envelope as missing its authenticator
        // (mixed-version fail-closed, contract §8).
        clientTransport: (clientEnd) =>
          tamperOutboundRequests(clientEnd, (doc) => {
            delete doc.signature;
            return doc;
          }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature"),
          details: { kind: "envelope_auth_missing" },
        });
        expect(host.stats.authRejections).toBe(1);
        expect(host.stats.invokesDispatched).toBe(0);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "fails the dial when the session snapshot signature is stripped (verify before establish)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const pair = loopbackTransportPair();
      const host = await startLoopbackHost({
        transport: pair.server,
        seed: seed(0xa0),
        clientPubkey: getPublicKeyEd25519(seed(0x10)),
        allowlist: [
          derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
        ],
        adapter: hostAdapter,
      });
      try {
        const stripping: Transport = {
          send: (envelope) => pair.client.send(envelope),
          recv: async () => {
            const bytes = await pair.client.recv();
            const doc = decodeWire(bytes);
            if ("initiator_peer_id" in doc) {
              delete doc.signature;
              return encodeWire(doc);
            }
            return bytes;
          },
          close: () => pair.client.close(),
        };
        await expect(
          connectRemoteAdapter({
            transport: stripping,
            localIdentity: { seed: seed(0x10) },
            localManifest: clientManifest(),
            remotePubkey: getPublicKeyEd25519(seed(0xa0)),
            allowlist: [
              derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0))),
            ],
          }),
        ).rejects.toThrow(/missing a signature/);
      } finally {
        host.close();
      }
    },
    15000,
  );
});

function sampleExtractRequestAdapter(runId = "run-adapter-ke"): ExtractRequest {
  return {
    run_id: runId,
    sources: [
      {
        schema_version: 1,
        source_id: "src-adapter",
        extensions: {},
      },
    ],
  };
}

function manifestWithExtract(hostId: string): HostCapabilityManifest {
  const base = schemaConformantManifest();
  const caps = [...new Set([...base.capabilities, CAPABILITY_KE_EXTRACTION])] as [
    string,
    ...string[],
  ];
  return { ...base, host_id: hostId, capabilities: caps };
}

describe("ke remote", () => {
  it("rejects malformed extract success payloads with INTERNAL_ERROR transport kind", async () => {
    const seedResponder = seed(0xb0);
    const seedClient = seed(0x20);
    const pubkeyResponder = getPublicKeyEd25519(seedResponder);
    const pubkeyClient = getPublicKeyEd25519(seedClient);
    const peerIdResponder = derivePeerIdFromEd25519Pubkey(pubkeyResponder);
    const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);
    const pair = loopbackTransportPair();
    const ports = {
      ...asBaselineOnly(new ToyWorldAdapter()),
      extract: async () =>
        spokeOk({ not_extract: true } as unknown as {
          candidates: never[];
          run: { run_id: string };
        }),
    };
    const responder = await connectResponder({
      transport: pair.server,
      identity: { seed: seedResponder },
      manifest: manifestWithExtract("responder-malformed"),
      allowlist: [peerIdClient],
      peerKeys: { [peerIdClient]: pubkeyClient },
      ports,
    });
    const client = await connectRemoteAdapter({
      transport: pair.client,
      localIdentity: { seed: seedClient },
      localManifest: manifestWithExtract("client-malformed"),
      remotePubkey: pubkeyResponder,
      allowlist: [peerIdResponder],
    });
    try {
      const result = await client.extract(sampleExtractRequestAdapter());
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
      expect(result.details?.kind).toBe("transport");
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });

  it("rejects extract success payloads with unknown top-level keys", () => {
    expect(
      isValidSuccessPayload("extract", {
        candidates: [],
        run: { run_id: "run-1" },
        junk: true,
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("extract", {
        candidates: [],
        run: { run_id: "run-1" },
      }),
    ).toBe(true);
  });

  it("rejects extract success payloads whose candidates are not objects", () => {
    expect(
      isValidSuccessPayload("extract", {
        candidates: ["not-an-object"],
        run: { run_id: "run-1" },
      }),
    ).toBe(false);
    expect(
      isValidSuccessPayload("extract", {
        candidates: [
          {
            schema_version: 1,
            entry_id: "e1",
            entry_type: "note",
            canonical_name: "n",
            status: "provisional",
            body: {},
            extensions: {},
          },
        ],
        run: { run_id: "run-1", method: "demo" },
      }),
    ).toBe(true);
  });

  it("rejects malformed extract success payloads with INTERNAL_ERROR transport kind for non-object candidates on the wire", async () => {
    const seedResponder = seed(0xb1);
    const seedClient = seed(0x21);
    const pubkeyResponder = getPublicKeyEd25519(seedResponder);
    const pubkeyClient = getPublicKeyEd25519(seedClient);
    const peerIdResponder = derivePeerIdFromEd25519Pubkey(pubkeyResponder);
    const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);
    const pair = loopbackTransportPair();
    const ports = {
      ...asBaselineOnly(new ToyWorldAdapter()),
      extract: async () =>
        spokeOk({
          candidates: ["bad"],
          run: { run_id: "run-adapter-ke" },
        } as unknown as {
          candidates: never[];
          run: { run_id: string };
        }),
    };
    const responder = await connectResponder({
      transport: pair.server,
      identity: { seed: seedResponder },
      manifest: manifestWithExtract("responder-malformed-candidate"),
      allowlist: [peerIdClient],
      peerKeys: { [peerIdClient]: pubkeyClient },
      ports,
    });
    const client = await connectRemoteAdapter({
      transport: pair.client,
      localIdentity: { seed: seedClient },
      localManifest: manifestWithExtract("client-malformed-candidate"),
      remotePubkey: pubkeyResponder,
      allowlist: [peerIdResponder],
    });
    try {
      const result = await client.extract(sampleExtractRequestAdapter());
      expect(result.ok).toBe(false);
      if (result.ok) return;
      expect(result.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
      expect(result.details?.kind).toBe("transport");
    } finally {
      client.close();
      responder.close();
      pair.client.close();
      pair.server.close();
    }
  });
});