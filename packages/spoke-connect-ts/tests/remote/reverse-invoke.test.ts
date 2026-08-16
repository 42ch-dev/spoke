/**
 * Reverse tool invocation + RemoteAdapter tool-serving mode (Task 1 of
 * tools-connect-reverse-invoke) — loopback pair: a tool-serving
 * `RemoteAdapter` (dialer) ↔ a `MinimalResponder` (responder) that issues
 * reverse invokes and serves the dialer's forward invokes.
 *
 * Asserts, per the frozen serving pipeline (`tool-contracts.md` §4/§6):
 * (a) request-first classification — an `op`-bearing doc is never a
 *     response, even though it carries the same correlation echo fields +
 *     payload as a response success branch (misclassification regression);
 * (b) the reverse-invoke pipeline: peek → verify → advance → gate →
 *     handler → signed response, with auth-before-advance (a failed verify
 *     leaves the inbound counter unchanged and the session usable);
 * (c) the deny-code matrix: gate fail / no handler → `op_unsupported`,
 *     sequence gap → `invalid_sequence`, signature failure → `auth_failed`;
 * (d) the public `invokeTool` forward face: happy path, deny mapping, and
 *     the `{ result }` success-payload gate.
 */

import { describe, expect, it } from "vitest";

import type {
  ConnectInvokeRequest,
  HostCapabilityManifest,
  ToolDescriptor,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  validateManifestTools,
} from "@42ch/spoke-operations";
import {
  connectRemoteAdapter,
  isConnectInvokeRequest,
  isConnectInvokeResponse,
  loopbackTransportPair,
  RemoteAdapter,
  type EnvelopeBytes,
  type Transport,
} from "@42ch/spoke-connect/remote";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { schemaConformantManifest } from "../../src/golden.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  startMinimalResponder,
  type MinimalResponder,
  type TestToolHandler,
} from "./minimal-responder.js";

/** Fixture seed: base+i, all values within byte range for base ≤ 0xe0. */
function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

const ADD_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.math.add",
  op: "tools.math.add",
  description: "Add two numbers",
  input: {},
  output: {},
};

const ECHO_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.echo.echo",
  op: "tools.echo.echo",
  description: "Echo the arguments back",
  input: {},
  output: {},
};

const BOOM_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.echo.boom",
  op: "tools.echo.boom",
  description: "Handler that throws",
  input: {},
  output: {},
};

/** Tool-carrying manifest: namespaces own the tool namespaces; every tool capability ∈ capabilities[]. */
function toolManifest(hostId: string): HostCapabilityManifest {
  return {
    ...schemaConformantManifest(),
    host_id: hostId,
    namespaces: ["toy_world", "math", "echo"],
    capabilities: [
      "spoke-baseline",
      "tools.math.add",
      "tools.echo.echo",
      "tools.echo.boom",
    ],
    tools: [ADD_DESCRIPTOR, ECHO_DESCRIPTOR, BOOM_DESCRIPTOR],
  };
}

/** The add handler used by most fixtures. */
function addHandler(calls: { args: Record<string, unknown> }[]): TestToolHandler {
  return async (args) => {
    calls.push({ args });
    return spokeOk({ sum: (args.a as number) + (args.b as number) });
  };
}

interface DialToolsOptions {
  clientManifest?: HostCapabilityManifest;
  responderManifest?: HostCapabilityManifest;
  invokeTimeoutMs?: number;
  clientTransport?: (clientEnd: Transport) => Transport;
  responderTransport?: (serverEnd: Transport) => Transport;
  responseOverride?: (request: ConnectInvokeRequest) => unknown;
}

async function dialWithTools(
  options: DialToolsOptions = {},
): Promise<{
  client: RemoteAdapter;
  responder: MinimalResponder;
  pair: ReturnType<typeof loopbackTransportPair>;
  peerIdResponder: string;
  peerIdClient: string;
}> {
  const seedResponder = seed(0xa0);
  const seedClient = seed(0x10);
  const pubkeyResponder = getPublicKeyEd25519(seedResponder);
  const pubkeyClient = getPublicKeyEd25519(seedClient);
  const peerIdResponder = derivePeerIdFromEd25519Pubkey(pubkeyResponder);
  const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

  const clientManifest = options.clientManifest ?? toolManifest("test-client");
  const responderManifest =
    options.responderManifest ?? toolManifest("test-responder");
  // Fixture hygiene: both manifests satisfy the manifest-tools rules
  // (descriptor validity, capability membership, namespace ownership).
  for (const manifest of [clientManifest, responderManifest]) {
    const validated = validateManifestTools(manifest);
    expect(validated.ok, `manifest tools must validate: ${manifest.host_id}`).toBe(true);
  }

  const pair = loopbackTransportPair();
  const responder = await startMinimalResponder({
    transport: options.responderTransport?.(pair.server) ?? pair.server,
    seed: seedResponder,
    clientPubkey: pubkeyClient,
    allowlist: [peerIdClient],
    manifest: responderManifest,
    invokeTimeoutMs: options.invokeTimeoutMs,
    responseOverride: options.responseOverride,
  });
  const client = await connectRemoteAdapter({
    transport: options.clientTransport?.(pair.client) ?? pair.client,
    localIdentity: { seed: seedClient },
    localManifest: clientManifest,
    remotePubkey: pubkeyResponder,
    allowlist: [peerIdResponder],
    invokeTimeoutMs: options.invokeTimeoutMs,
  });
  return { client, responder, pair, peerIdResponder, peerIdClient };
}

/** Decode wire bytes to a JSON document (wire-level injector fixtures). */
function decodeWire(bytes: EnvelopeBytes): Record<string, unknown> {
  return decodeJsonMessage(bytes) as Record<string, unknown>;
}

/** Re-encode a mutated document as wire bytes (wire-level injector fixtures). */
function encodeWire(doc: unknown): EnvelopeBytes {
  return new TextEncoder().encode(encodeJsonMessage(doc));
}

describe("RemoteAdapter tool serving over reverse invokes", () => {
  it(
    "serves a reverse invoke (responder issues → dialer handler runs → signed result returns)",
    async () => {
      const calls: { args: Record<string, unknown> }[] = [];
      const { client, responder } = await dialWithTools();
      try {
        client.registerToolHandler("tools.math.add", addHandler(calls));

        const result = await responder.issueInvoke("tools.math.add", {
          a: 2,
          b: 3,
        });
        expect(result).toEqual({ ok: true, value: { sum: 5 } });
        // The dialer-side registered handler is what ran (not a responder
        // side effect), with the request's arguments object passed through.
        expect(calls).toEqual([{ args: { a: 2, b: 3 } }]);
        expect(responder.stats.reverseInvokesIssued).toBe(1);
        expect(responder.stats.responsesVerified).toBe(1);
        // The session stays Established on both ends.
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "classifies request shape before response shape — an op-bearing doc is never a response",
    async () => {
      // Guard level: a reverse request carries the same correlation echo
      // fields + payload as a response success branch; the hardened
      // response discriminator must reject ANY op-bearing doc.
      const requestShaped: Record<string, unknown> = {
        session_id: "s",
        sequence: 0,
        request_id: "r",
        op: "tools.math.add",
        payload: { arguments: {} },
        signature: "x",
        extensions: {},
      };
      expect(isConnectInvokeRequest(requestShaped)).toBe(true);
      expect(isConnectInvokeResponse(requestShaped)).toBe(false);
      const responseShapedWithOp: Record<string, unknown> = {
        session_id: "s",
        sequence: 0,
        request_id: "r",
        op: "tools.math.add",
        payload: { result: 1 },
        signature: "x",
        extensions: {},
      };
      expect(isConnectInvokeResponse(responseShapedWithOp)).toBe(false);
    },
  );

  it(
    "does not demux a reverse request as a response while a forward invoke waiter is pending",
    async () => {
      const calls: { args: Record<string, unknown> }[] = [];
      const { client, responder } = await dialWithTools();
      try {
        client.registerToolHandler("tools.math.add", addHandler(calls));
        // The responder serves the dialer's forward invoke; the dialer
        // serves the responder's reverse invoke. Under the pre-fix
        // discriminator the reverse request (op-bearing, response-shaped)
        // would be swallowed by the request_id demux and the responder
        // would time out.
        responder.registerToolHandler("tools.echo.echo", async (args) =>
          spokeOk(args),
        );
        const [forward, reverse] = await Promise.all([
          client.invokeTool("tools.echo.echo", { v: 1 }),
          responder.issueInvoke("tools.math.add", { a: 10, b: 32 }),
        ]);
        expect(reverse).toEqual({ ok: true, value: { sum: 42 } });
        expect(forward).toEqual({ ok: true, value: { v: 1 } });
        expect(calls).toEqual([{ args: { a: 10, b: 32 } }]);
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "denies a reverse invoke with op_unsupported when no handler is registered (fail-closed serving)",
    async () => {
      const { client, responder } = await dialWithTools();
      try {
        // The tool IS negotiated (both manifests list it) but no handler
        // is registered — gate passes, serving fails closed.
        const result = await responder.issueInvoke("tools.math.add", {
          a: 1,
          b: 2,
        });
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("no handler registered"),
          details: { wire_code: "op_unsupported" },
        });
        // The session stays usable: a registered handler serves the next
        // reverse invoke.
        client.registerToolHandler("tools.math.add", async (args) =>
          spokeOk({ sum: (args.a as number) + (args.b as number) }),
        );
        const retry = await responder.issueInvoke("tools.math.add", {
          a: 4,
          b: 5,
        });
        expect(retry).toEqual({ ok: true, value: { sum: 9 } });
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "denies a reverse invoke with op_unsupported when the tool is not negotiated (dispatch gate)",
    async () => {
      // The client manifest carries no tools: the negotiated set lacks the
      // tool capability, so the client's dispatch gate denies the invoke
      // (frozen deny matrix: gate fail → op_unsupported).
      const noTools: HostCapabilityManifest = {
        ...schemaConformantManifest(),
        host_id: "test-client",
        capabilities: ["spoke-baseline"],
      };
      const { client, responder } = await dialWithTools({
        clientManifest: noTools,
      });
      try {
        const result = await responder.issueInvoke("tools.math.add", {
          a: 1,
          b: 2,
        });
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("not authorized"),
          details: { wire_code: "op_unsupported" },
        });
        expect(responder.stats.responsesVerified).toBe(1);
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "rejects a tampered reverse request with auth_failed and does NOT advance the inbound counter",
    async () => {
      const calls: { args: Record<string, unknown> }[] = [];
      let tampered = false;
      // Wire-level injector on the responder end: mutate the FIRST outbound
      // reverse request's payload after signing (the view an active
      // transport attacker has on the client end). The handshake hellos
      // pass through unchanged.
      const { client, responder } = await dialWithTools({
        responderTransport: (serverEnd) => ({
          send: async (envelope) => {
            const doc = decodeWire(envelope);
            if ("op" in doc && !tampered) {
              tampered = true;
              (doc.payload as Record<string, unknown>).tampered = true;
              await serverEnd.send(encodeWire(doc));
              return;
            }
            await serverEnd.send(envelope);
          },
          recv: () => serverEnd.recv(),
          close: () => serverEnd.close?.(),
        }),
      });
      try {
        client.registerToolHandler("tools.math.add", addHandler(calls));

        // Tampered request (sequence 0): envelope-auth verify fails BEFORE
        // advance — the error branch is auth_failed with the locked
        // details.kind, and no handler side effect runs.
        const tamperedResult = await responder.issueInvoke("tools.math.add", {
          a: 1,
          b: 2,
        }, 0);
        expect(tamperedResult).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature does not verify"),
          details: { kind: "envelope_auth_invalid" },
        });
        expect(calls).toEqual([]);

        // Auth-before-advance: the client's inbound counter is UNCHANGED
        // (still expects 0), so re-issuing with the same sequence succeeds
        // and the handler runs — the session stayed usable and no state
        // was mutated by the forged envelope.
        const retry = await responder.issueInvoke("tools.math.add", {
          a: 20,
          b: 22,
        }, 0);
        expect(retry).toEqual({ ok: true, value: { sum: 42 } });
        expect(calls).toEqual([{ args: { a: 20, b: 22 } }]);
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "rejects a sequence-gap reverse invoke with invalid_sequence and does NOT advance the counter",
    async () => {
      const calls: { args: Record<string, unknown> }[] = [];
      const { client, responder } = await dialWithTools();
      try {
        client.registerToolHandler("tools.math.add", addHandler(calls));

        // The responder jumps to sequence 5; the client expects 0. The
        // peek fails — error branch invalid_sequence, counter unchanged,
        // no handler side effect.
        const gap = await responder.issueInvoke("tools.math.add", { a: 1, b: 1 }, 5);
        expect(gap.ok).toBe(false);
        expect(gap).toMatchObject({
          code: SpokeRejectCode.INVALID_INPUT,
          details: { wire_code: "invalid_sequence" },
        });
        expect(calls).toEqual([]);

        // The inbound counter is still at 0: the next expected sequence
        // succeeds.
        const retry = await responder.issueInvoke("tools.math.add", { a: 40, b: 2 }, 0);
        expect(retry).toEqual({ ok: true, value: { sum: 42 } });
        expect(calls).toEqual([{ args: { a: 40, b: 2 } }]);
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "answers the error branch when a handler throws, without loop damage",
    async () => {
      const boomCalls: string[] = [];
      const { client, responder } = await dialWithTools();
      try {
        client.registerToolHandler("tools.echo.boom", async () => {
          boomCalls.push("boom");
          throw new Error("provider exploded");
        });
        client.registerToolHandler("tools.math.add", async (args) =>
          spokeOk({ sum: (args.a as number) + (args.b as number) }),
        );

        // A throwing handler answers the error branch (mapped via
        // toErrorEnvelope → INTERNAL_ERROR) instead of crashing the loop.
        const thrown = await responder.issueInvoke("tools.echo.boom", {});
        expect(thrown).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: "provider exploded",
        });
        expect(boomCalls).toEqual(["boom"]);

        // Loop damage check: the receive loop survived the throw — a
        // different reverse invoke for a healthy handler still succeeds.
        const healthy = await responder.issueInvoke("tools.math.add", { a: 40, b: 2 });
        expect(healthy).toEqual({ ok: true, value: { sum: 42 } });
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "answers the error branch with the handler's SpokeReject code (mapped via toErrorEnvelope)",
    async () => {
      const { client, responder } = await dialWithTools();
      try {
        client.registerToolHandler("tools.echo.echo", async () =>
          spokeReject(
            SpokeRejectCode.REVISION_CONFLICT,
            "the tool's backing store has a newer revision",
          ),
        );
        const result = await responder.issueInvoke("tools.echo.echo", { v: 1 });
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.REVISION_CONFLICT,
          message: "the tool's backing store has a newer revision",
        });
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "forwards a tool invoke via invokeTool (dialer → responder handler → signed result)",
    async () => {
      const responderCalls: { args: Record<string, unknown> }[] = [];
      const { client, responder } = await dialWithTools();
      try {
        responder.registerToolHandler("tools.math.add", addHandler(responderCalls));
        const result = await client.invokeTool("tools.math.add", { a: 21, b: 21 });
        expect(result).toEqual({ ok: true, value: { sum: 42 } });
        expect(responderCalls).toEqual([{ args: { a: 21, b: 21 } }]);
        expect(responder.stats.handlersRun).toBe(1);
        expect(responder.stats.sequenceRejections).toBe(0);
        expect(responder.stats.authRejections).toBe(0);
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "maps a forward invokeTool deny to CAPABILITY_PORT_MISSING with the wire code preserved",
    async () => {
      const { client, responder } = await dialWithTools();
      try {
        // tools.echo.boom is negotiated but the responder serves no handler
        // for it — fail-closed deny → op_unsupported → D7 mapping.
        const result = await client.invokeTool("tools.echo.boom", {});
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("no handler registered"),
          details: { wire_code: "op_unsupported" },
        });
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "rejects a forward invokeTool whose success payload lacks a result key (INTERNAL_ERROR kind transport)",
    async () => {
      // The responder answers a success envelope whose payload is not
      // `{ result: <opaque JSON> }` — the frozen tool success-payload gate
      // must reject instead of surfacing spokeOk(garbage).
      const { client, responder } = await dialWithTools({
        responseOverride: (request) => ({
          session_id: request.session_id,
          sequence: request.sequence,
          request_id: request.request_id,
          payload: { garbage: true },
          extensions: {},
        }),
      });
      try {
        const result = await client.invokeTool("tools.math.add", { a: 1, b: 2 });
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("payload decode failed"),
          details: { kind: "transport" },
        });
        // The malformed payload fails only this waiter — the session stays
        // usable.
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );

  it(
    "fails fast with INVALID_INPUT on a non-tool capability id (invokeTool grammar gate)",
    async () => {
      const { client, responder } = await dialWithTools();
      try {
        const result = await client.invokeTool("spoke-baseline", {});
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INVALID_INPUT,
          message: expect.stringContaining('must start with "tools." prefix'),
          details: { capability_id: "spoke-baseline" },
        });
        // No wire traffic: the grammar gate is local.
        expect(responder.stats.reverseInvokesIssued).toBe(0);
      } finally {
        client.close();
        responder.close();
      }
    },
    15000,
  );
});
