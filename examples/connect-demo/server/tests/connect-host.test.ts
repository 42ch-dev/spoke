/**
 * ConnectHost integration tests — a real `connectRemoteAdapter`
 * (`@42ch/spoke-connect/remote`) dialing the demo responder over
 * `loopbackTransportPair()`. The library client enforces protocol_version 2
 * strictly (session snapshot verify at establish, envelope-auth verify on
 * every response), so green tests prove signature interop in both
 * directions: client → host (invoke requests) and host → client (hello,
 * session snapshot, responses).
 *
 * Wire phases under test mirror `loopback-host.ts`: allowlist fail-closed
 * check FIRST in the handshake; per-invoke gate = sequence peek
 * (non-mutating) → envelope-auth verify → advance; dispatch gate via the
 * product `op_capability_requirements` map (`port.*` → `spoke-baseline`);
 * responses signed `spoke-connect-invoke-response-jcs-v1`.
 */

import { describe, expect, it } from "vitest";

import canonicalize from "canonicalize";

import type {
  ConnectHello,
  ConnectInvokeRequest,
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  Relation,
} from "@42ch/spoke-schemas";
import { SpokeRejectCode } from "@42ch/spoke-operations";
import {
  base64UrlEncode,
  decodeJsonMessage,
  encodeJsonMessage,
  generateNonce,
  signEd25519,
  signHelloEd25519,
  verifyHelloEd25519,
} from "@42ch/spoke-connect";
import {
  connectRemoteAdapter,
  loopbackTransportPair,
  type EnvelopeBytes,
  type RemoteAdapter,
  type Transport,
} from "@42ch/spoke-connect/remote";

import { DEMO_SERVER_MANIFEST, MockAdapter } from "../src/adapter/mock-adapter.js";
import {
  DEMO_SEED_ENTRIES,
  DEMO_SEED_RELATIONS,
  DEMO_SEED_RULES,
  DEMO_SCOPE_ID,
} from "../src/engine/seed-corpus.js";
import { ConnectHost } from "../src/host/connect-host.js";
import {
  DEMO_CLIENT_PEER_ID,
  DEMO_CLIENT_PUBKEY,
  DEMO_CLIENT_SEED,
  DEMO_SERVER_PEER_ID,
  DEMO_SERVER_PUBKEY,
  DEMO_SERVER_SEED,
  DEMO_STRANGER_PEER_ID,
  DEMO_STRANGER_PUBKEY,
  DEMO_STRANGER_SEED,
} from "../src/identities.js";

const textEncoder = new TextEncoder();

/** The demo client manifest (third-party flow, T3). */
const CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-client",
  roles: ["checker"],
  capabilities: ["spoke-baseline"],
  namespaces: [DEMO_SCOPE_ID],
  extensions: {},
};

/**
 * Preconfigured client public keys by peer_id (key distribution is
 * transport-adapter-owned per the spec). The stranger's key is known but
 * deliberately NOT on the server allowlist — the negative proof dials with
 * the stranger identity and is rejected server-side.
 */
const PEER_KEYS: Record<string, Uint8Array> = {
  [DEMO_CLIENT_PEER_ID]: DEMO_CLIENT_PUBKEY,
  [DEMO_STRANGER_PEER_ID]: DEMO_STRANGER_PUBKEY,
};

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

function decodeEnvelope(bytes: EnvelopeBytes): unknown {
  return decodeJsonMessage(bytes);
}

/** Start a ConnectHost serving a fresh MockAdapter on the server end. */
function startHost(): { host: ConnectHost; client: Transport } {
  const { client, server } = loopbackTransportPair();
  const host = new ConnectHost({
    seed: DEMO_SERVER_SEED,
    manifest: DEMO_SERVER_MANIFEST,
    allowlist: [DEMO_CLIENT_PEER_ID],
    peerKeys: PEER_KEYS,
    adapter: new MockAdapter(),
  });
  host.attach(server);
  return { host, client };
}

/** Dial with the REAL library client (`connectRemoteAdapter`, v2 strict). */
async function dialRealClient(
  client: Transport,
  seed: Uint8Array = DEMO_CLIENT_SEED,
): Promise<RemoteAdapter> {
  return connectRemoteAdapter({
    transport: client,
    localIdentity: { seed },
    localManifest: CLIENT_MANIFEST,
    remotePubkey: DEMO_SERVER_PUBKEY,
    allowlist: [DEMO_SERVER_PEER_ID],
  });
}

/** Sign a wire `ConnectInvokeRequest` over the locked 5-field set (raw client). */
async function signInvokeRequestTest(
  request: {
    session_id: string;
    sequence: number;
    request_id: string;
    op: string;
    payload: Record<string, unknown>;
  },
): Promise<ConnectInvokeRequest> {
  const jcs = canonicalize({
    session_id: request.session_id,
    sequence: request.sequence,
    request_id: request.request_id,
    op: request.op,
    payload: request.payload,
  });
  if (jcs === undefined) {
    throw new Error("payload is not JSON-serializable");
  }
  return {
    ...request,
    signature: base64UrlEncode(
      await signEd25519(DEMO_CLIENT_SEED, textEncoder.encode(jcs)),
    ),
    extensions: {},
  };
}

describe("ConnectHost handshake", () => {
  it("establishes a session with a real connectRemoteAdapter dial", async () => {
    const { host, client } = startHost();
    const adapter = await dialRealClient(client);

    expect(adapter.state).toBe("Established");
    expect(adapter.remotePeerId).toBe(DEMO_SERVER_PEER_ID);
    expect(adapter.sessionId).toBe(host.sessionId);
    expect(adapter.sessionId.length).toBeGreaterThan(0);
    expect(adapter.remoteManifest).toEqual(DEMO_SERVER_MANIFEST);
    expect(host.stats.hellosVerified).toBe(1);

    adapter.close();
    host.close();
  });

  it("rejects a non-allowlisted peer during the handshake (fail-closed)", async () => {
    const { host, client } = startHost();

    // The stranger's own allowlist trusts the server, so the dial is
    // attempted; the SERVER-side allowlist rejects the hello and closes the
    // transport, failing the dial fast.
    await expect(dialRealClient(client, DEMO_STRANGER_SEED)).rejects.toThrow();
    expect(host.stats.hellosVerified).toBe(0);

    host.close();
  });
});

describe("ConnectHost port-op dispatch (D4 catalogue)", () => {
  it("round-trips all nine port ops through the host into the adapter", async () => {
    const { host, client } = startHost();
    const adapter = await dialRealClient(client);

    // port.knowledge.get — seed entry round-trip.
    const mira = await adapter.getKnowledgeEntry(DEMO_SEED_ENTRIES[0].entry_id);
    expect(mira.ok).toBe(true);
    if (mira.ok) {
      expect(mira.value).toEqual(DEMO_SEED_ENTRIES[0]);
    }

    // port.knowledge.put — create (expected_base_revision null), then a
    // compare-and-swap update (expected_base_revision 1) over the wire.
    const compass: KnowledgeEntry = {
      schema_version: 1,
      entry_id: "demo-harbor/item/compass",
      entry_type: "item",
      canonical_name: "Compass",
      status: "provisional",
      body: { summary: "A brass compass." },
      extensions: {},
    };
    const created = await adapter.putKnowledgeEntry(compass, null);
    expect(created.ok).toBe(true);
    if (created.ok) {
      expect(created.value.revision).toBe(1);
    }
    const updated = await adapter.putKnowledgeEntry(
      { ...compass, status: "confirmed" },
      1,
    );
    expect(updated.ok).toBe(true);
    if (updated.ok) {
      expect(updated.value.revision).toBe(2);
    }
    const fetched = await adapter.getKnowledgeEntry(compass.entry_id);
    expect(fetched.ok).toBe(true);
    if (fetched.ok) {
      expect(fetched.value.revision).toBe(2);
    }

    // Negative OCC over the wire: a future base revision rejects
    // REVISION_CONFLICT through the host's error branch.
    const conflicted = await adapter.putKnowledgeEntry(
      { ...compass, status: "confirmed" },
      99,
    );
    expect(conflicted.ok).toBe(false);
    if (!conflicted.ok) {
      expect(conflicted.code).toBe(SpokeRejectCode.REVISION_CONFLICT);
    }

    // port.relation.get / put.
    const seedRelation = DEMO_SEED_RELATIONS[0];
    const gotRelation = await adapter.getRelation(seedRelation.relation_id);
    expect(gotRelation.ok).toBe(true);
    if (gotRelation.ok) {
      expect(gotRelation.value).toEqual(seedRelation);
    }
    const newRelation: Relation = {
      schema_version: 1,
      relation_id: "demo-harbor/relation/compass-located-in-harbor",
      relation_type: "located_in",
      from_id: compass.entry_id,
      to_id: "demo-harbor/location/harbor",
      extensions: {},
    };
    const putRelation = await adapter.putRelation(newRelation, null);
    expect(putRelation.ok).toBe(true);
    if (putRelation.ok) {
      expect(putRelation.value.revision).toBe(1);
    }

    // port.scope.list_knowledge_entries — seeds + derived digest + submitted.
    const listed = await adapter.listKnowledgeEntries({
      scope_id: DEMO_SCOPE_ID,
    });
    expect(listed.ok).toBe(true);
    if (listed.ok) {
      const ids = listed.value.map((entry) => entry.entry_id).sort();
      expect(ids).toEqual(
        [
          ...DEMO_SEED_ENTRIES.map((entry) => entry.entry_id),
          "derived/world-digest",
          compass.entry_id,
        ].sort(),
      );
    }

    // port.scope.list_timeline_events — the engine never emits events.
    const events = await adapter.listTimelineEvents({ scope_id: DEMO_SCOPE_ID });
    expect(events.ok).toBe(true);
    if (events.ok) {
      expect(events.value).toEqual([]);
    }

    // port.finding.put — round-trip.
    const finding: Finding = {
      schema_version: 1,
      finding_id: "demo-harbor/finding/compass-uncased",
      severity: "info",
      status: "open",
      title: "Compass uncased",
      description: "The compass has no case.",
      target_entry_id: compass.entry_id,
      extensions: {},
    };
    const putFindings = await adapter.putFindings([finding]);
    expect(putFindings.ok).toBe(true);
    if (putFindings.ok) {
      expect(putFindings.value).toEqual([finding]);
    }

    // port.rule.list — seed rule by ref.
    const rules = await adapter.listRules([DEMO_SEED_RULES[0].rule_id]);
    expect(rules.ok).toBe(true);
    if (rules.ok) {
      expect(rules.value).toHaveLength(1);
      expect(rules.value[0].canonical_name).toBe("No isolated entries");
    }

    // port.host.list_peer_manifests — the demo host knows no peers.
    const peers = await adapter.listPeerHostCapabilityManifests();
    expect(peers.ok).toBe(true);
    if (peers.ok) {
      expect(peers.value).toEqual([]);
    }

    // 12 successful dispatches: get, put, put-CAS, get, getRelation,
    // putRelation, listEntries, listEvents, putFindings, listRules,
    // listPeers + the OCC-conflict put (still dispatched).
    expect(host.stats.invokesDispatched).toBe(12);
    expect(host.stats.sequenceRejections).toBe(0);
    expect(host.stats.authRejections).toBe(0);
    expect(host.stats.dispatchDenials).toBe(0);

    adapter.close();
    host.close();
  });
});

describe("ConnectHost per-invoke gate (peek → verify → advance)", () => {
  it("fails only a tampered invoke request while the session stays usable", async () => {
    const { host, client } = startHost();

    // Raw initiator handshake (the library client is exercised in the other
    // tests; here the test drives the wire so it can read host responses).
    const initiatorNonce = generateNonce();
    await client.send(
      encodeEnvelope(
        await signHelloEd25519(DEMO_CLIENT_SEED, initiatorNonce, CLIENT_MANIFEST),
      ),
    );
    const serverHello = decodeEnvelope(await client.recv()) as ConnectHello;
    await verifyHelloEd25519(
      DEMO_SERVER_PUBKEY,
      DEMO_SERVER_PEER_ID,
      serverHello,
      initiatorNonce,
    );
    const sessionDoc = decodeEnvelope(await client.recv()) as {
      session_id: string;
      initial_sequence: number;
      initiator_peer_id: string;
      responder_peer_id: string;
      signature: string;
    };
    expect(sessionDoc.session_id).toBe(host.sessionId);
    expect(sessionDoc.initial_sequence).toBe(0);
    expect(sessionDoc.initiator_peer_id).toBe(DEMO_CLIENT_PEER_ID);
    expect(sessionDoc.responder_peer_id).toBe(DEMO_SERVER_PEER_ID);
    expect(sessionDoc.signature).toHaveLength(86);

    const sessionId = sessionDoc.session_id;
    const getPayload = (entryId: string) => ({
      session_id: sessionId,
      sequence: 0,
      request_id: "tampered-1",
      op: "port.knowledge.get",
      payload: { entry_id: entryId },
    });

    // 1. A wire-valid invoke with a garbage signature: the envelope-auth
    //    verify rejects it (`envelope_auth_invalid`) and the inbound counter
    //    is NOT advanced (no session-state mutation on auth failure).
    const tampered: ConnectInvokeRequest = {
      ...getPayload(DEMO_SEED_ENTRIES[0].entry_id),
      signature: "A".repeat(86), // passes presence + canonical + length, fails Ed25519
      extensions: {},
    };
    await client.send(encodeEnvelope(tampered));
    const authError = decodeEnvelope(await client.recv()) as {
      session_id: string;
      sequence: number;
      request_id: string;
      error: { code: string; details: { kind: string } };
      signature: string;
    };
    expect(authError.session_id).toBe(sessionId);
    expect(authError.sequence).toBe(0);
    expect(authError.request_id).toBe("tampered-1");
    expect(authError.error.code).toBe("auth_failed");
    expect(authError.error.details.kind).toBe("envelope_auth_invalid");
    expect(authError.signature).toHaveLength(86);

    // 2. A properly signed request at the SAME sequence (0) succeeds — the
    //    failed envelope did not consume the sequence position.
    const valid = await signInvokeRequestTest({
      ...getPayload(DEMO_SEED_ENTRIES[0].entry_id),
      request_id: "valid-after-tamper",
    });
    await client.send(encodeEnvelope(valid));
    const okResponse = decodeEnvelope(await client.recv()) as {
      payload: KnowledgeEntry;
      signature: string;
    };
    expect(okResponse.payload).toEqual(DEMO_SEED_ENTRIES[0]);
    expect(okResponse.signature).toHaveLength(86);

    // 3. The next sequence also works — the session stays fully usable.
    const next = await signInvokeRequestTest({
      session_id: sessionId,
      sequence: 1,
      request_id: "second-request",
      op: "port.knowledge.get",
      payload: { entry_id: DEMO_SEED_ENTRIES[1].entry_id },
    });
    await client.send(encodeEnvelope(next));
    const secondResponse = decodeEnvelope(await client.recv()) as {
      payload: KnowledgeEntry;
    };
    expect(secondResponse.payload).toEqual(DEMO_SEED_ENTRIES[1]);

    expect(host.stats.authRejections).toBe(1);
    expect(host.stats.invokesDispatched).toBe(2);
    expect(host.stats.sequenceRejections).toBe(0);

    host.close();
  });
});
