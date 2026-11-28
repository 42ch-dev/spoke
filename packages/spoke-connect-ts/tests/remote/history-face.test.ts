import { readFileSync } from "node:fs";

import canonicalize from "canonicalize";
import { describe, expect, it, vi } from "vitest";

import {
  spokeOk,
  spokeReject,
  type HistoryPort,
  type SpokeResult,
} from "@42ch/spoke-operations";

import type {
  HostCapabilityManifest,
  Principal,
  RevisionSnapshot,
  Scope,
} from "@42ch/spoke-schemas";

import {
  connectRemoteAdapter,
  connectResponder,
  loopbackTransportPair,
} from "../../src/remote/index.js";
import {
  decodeHistoryCall,
  HistoryPortService,
  InMemoryHistoryDedupStore,
  type RemoteAdapter,
  type RemoteHistoryService,
} from "../../src/remote/index.js";
import { isValidRevisionSnapshot } from "../../src/remote/payload.js";
import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";

function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

function manifest(
  hostId: string,
  capabilities: string[],
): HostCapabilityManifest {
  return {
    schema_version: 1,
    host_id: hostId,
    roles: ["data-store"],
    capabilities: capabilities as [string, ...string[]],
    namespaces: ["world_a"],
    extensions: {},
  };
}

const SCOPE_A: Scope = { scope_id: "scope_a" };
const SCOPE_B: Scope = { scope_id: "scope_b" };
const PRINCIPAL_A: Principal = { principal_id: "peer-a", extensions: {} };

function snapshot(
  entityId: string,
  revision: number,
  extras: Record<string, unknown> = {},
): RevisionSnapshot {
  return {
    schema_version: 1,
    entity_kind: "knowledge_entry",
    entity_id: entityId,
    revision,
    snapshot: {
      schema_version: 1,
      entry_id: entityId,
      entry_type: "note",
      canonical_name: entityId,
      status: "confirmed",
      revision,
      body: {},
      extensions: {},
      ...extras,
    } as RevisionSnapshot["snapshot"],
    extensions: {},
  };
}

function admission(
  scope: Scope = SCOPE_A,
  principal: Principal = PRINCIPAL_A,
) {
  return (_peerId: string): SpokeResult<{ principal: Principal; scope: Scope }> =>
    spokeOk({ principal, scope });
}

function countingPort(handlers: {
  get?: HistoryPort["getHistorySnapshot"];
  prune?: HistoryPort["pruneHistory"];
}): { port: HistoryPort; calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    port: {
      getHistorySnapshot:
        handlers.get ??
        (async (principal, scope, kind, id, revision) => {
          calls.push(`get:${id}@${revision}`);
          return spokeOk(snapshot(id, revision));
        }),
      pruneHistory:
        handlers.prune ??
        (async (principal, scope, kind, id, keep) => {
          calls.push(`prune:${id}@${keep}`);
          return spokeOk({ deleted: 1, floor_revision: 2 });
        }),
    },
  };
}

describe("decodeHistoryCall — closed request shape", () => {
  const base = {
    entity_kind: "knowledge_entry",
    entity_id: "ke_1",
    revision: 3,
  };

  it("rejects undeclared top-level members", () => {
    const result = decodeHistoryCall("history.revisions.get", {
      ...base,
      bogus: 1,
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe("INVALID_INPUT");
    }
  });

  it("rejects non-object payloads", () => {
    const result = decodeHistoryCall("history.revisions.get", [base]);
    expect(result.ok).toBe(false);
  });

  it("rejects empty entity_kind / entity_id", () => {
    for (const payload of [
      { ...base, entity_kind: "" },
      { ...base, entity_id: "" },
      { ...base, entity_kind: 1 },
    ]) {
      const result = decodeHistoryCall("history.revisions.get", payload);
      expect(result.ok).toBe(false);
    }
  });

  it("enforces the inclusive [0, 2^53] revision domain", () => {
    for (const payload of [
      { ...base, revision: 2 ** 53 },
      { ...base, revision: 0 },
    ]) {
      expect(decodeHistoryCall("history.revisions.get", payload).ok).toBe(true);
    }
    for (const revision of [2 ** 53 + 2, 1.5, -1, "3"]) {
      const result = decodeHistoryCall("history.revisions.get", {
        ...base,
        revision,
      });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe("INVALID_INPUT");
      }
    }
    expect(
      decodeHistoryCall("history.revisions.prune", {
        entity_kind: "knowledge_entry",
        entity_id: "ke_1",
        keep: 2 ** 53,
        idempotency_key: "k1",
      }).ok,
    ).toBe(true);
    for (const keep of [2 ** 53 + 2, 2.5, -1]) {
      expect(
        decodeHistoryCall("history.revisions.prune", {
          entity_kind: "knowledge_entry",
          entity_id: "ke_1",
          keep,
          idempotency_key: "k1",
        }).ok,
      ).toBe(false);
    }
  });

  it("enforces the SchemaVersion [1, 2^53-1] ceiling on the snapshot envelope", () => {
    expect(
      isValidRevisionSnapshot({ ...snapshot("ke_1", 1), schema_version: 2 ** 53 - 1 }),
    ).toBe(true);
    expect(
      isValidRevisionSnapshot({ ...snapshot("ke_1", 1), schema_version: 2 ** 53 }),
    ).toBe(false);
  });

  it("enforces idempotency_key 1–128 chars on prune", () => {
    const payload = {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      keep: 2,
    };
    for (const key of [undefined, "", "k".repeat(129), 42]) {
      const result = decodeHistoryCall("history.revisions.prune", {
        ...payload,
        idempotency_key: key,
      });
      expect(result.ok).toBe(false);
    }
    expect(
      decodeHistoryCall("history.revisions.prune", {
        ...payload,
        idempotency_key: "k".repeat(128),
      }).ok,
    ).toBe(true);
    expect(
      decodeHistoryCall("history.revisions.prune", {
        ...payload,
        idempotency_key: "\u{1F4A7}".repeat(65),
      }).ok,
    ).toBe(true);
    expect(
      decodeHistoryCall("history.revisions.prune", {
        ...payload,
        idempotency_key: "\u{1F4A7}".repeat(129),
      }).ok,
    ).toBe(false);
  });

  it("rejects unknown members on prune and malformed scope", () => {
    expect(
      decodeHistoryCall("history.revisions.prune", {
        entity_kind: "knowledge_entry",
        entity_id: "ke_1",
        keep: 2,
        idempotency_key: "k1",
        extra: true,
      }).ok,
    ).toBe(false);
    expect(
      decodeHistoryCall("history.revisions.get", { ...base, scope: { nope: 1 } })
        .ok,
    ).toBe(false);
    expect(
      decodeHistoryCall("history.revisions.get", { ...base, scope: "s" }).ok,
    ).toBe(false);
  });

  it("decodes the validated arguments including the prune key", () => {
    const get = decodeHistoryCall("history.revisions.get", base);
    expect(get).toMatchObject({
      ok: true,
      value: { kind: "get", entityKind: "knowledge_entry", revision: 3 },
    });
    const prune = decodeHistoryCall("history.revisions.prune", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      keep: 2,
      idempotency_key: "key-1",
      scope: SCOPE_A,
    });
    expect(prune).toMatchObject({
      ok: true,
      value: { kind: "prune", keep: 2, idempotencyKey: "key-1" },
    });
  });
});

describe("HistoryPortService — admission + scope", () => {
  it("admission refusal propagates before any port call", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, () =>
      spokeReject("NOT_AUTHORIZED", "peer not admitted"),
    );
    const result = await service.invoke("peer-x", "history.revisions.get", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 1,
    });
    expect(result).toEqual({
      ok: false,
      code: "NOT_AUTHORIZED",
      message: "peer not admitted",
    });
    expect(calls).toHaveLength(0);
  });

  it("malformed input answers INVALID_INPUT without invoking admission", async () => {
    const seen: Array<string | undefined> = [];
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, (_peerId, scopeId) => {
      seen.push(scopeId);
      return spokeOk({ principal: PRINCIPAL_A, scope: SCOPE_A });
    });
    // Empty `entity_kind` fails the closed request decode: the documented
    // INVALID_INPUT must win, and a selector-aware callback must not run at
    // all (Rust parity: decode before admission).
    const result = await service.invoke("peer-a", "history.revisions.get", {
      entity_kind: "",
      entity_id: "ke_1",
      revision: 1,
      scope: { scope_id: "scope_a" },
    });
    expect(result).toMatchObject({ ok: false, code: "INVALID_INPUT" });
    expect(seen).toHaveLength(0);
    expect(calls).toHaveLength(0);
  });

  it("present scope must equal the admission — get answers NOT_FOUND, prune NOT_AUTHORIZED", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission(SCOPE_A));
    const get = await service.invoke("peer-a", "history.revisions.get", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 1,
      scope: SCOPE_B,
    });
    expect(get).toMatchObject({ ok: false, code: "NOT_FOUND" });
    const prune = await service.invoke("peer-a", "history.revisions.prune", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      keep: 1,
      idempotency_key: "k1",
      scope: SCOPE_B,
    });
    expect(prune).toMatchObject({ ok: false, code: "NOT_AUTHORIZED" });
    expect(calls).toHaveLength(0);
  });

  it("absent scope resolves to the admitted scope; equal scope is admitted", async () => {
    const seen: Array<{ principal: Principal; scope: Scope }> = [];
    const { port } = countingPort({
      get: async (principal, scope, _kind, id, revision) => {
        seen.push({ principal, scope });
        return spokeOk(snapshot(id, revision));
      },
    });
    const service = new HistoryPortService(port, admission(SCOPE_A));
    const ok = await service.invoke("peer-a", "history.revisions.get", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 1,
    });
    expect(ok).toMatchObject({ ok: true });
    const okEqual = await service.invoke("peer-a", "history.revisions.get", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 1,
      scope: { scope_id: "scope_a" },
    });
    expect(okEqual).toMatchObject({ ok: true });
    expect(seen).toEqual([
      { principal: PRINCIPAL_A, scope: SCOPE_A },
      { principal: PRINCIPAL_A, scope: SCOPE_A },
    ]);
  });

  it("passes the request's declared scope selector to admission", async () => {
    const seen: Array<string | undefined> = [];
    const { port } = countingPort({});
    const service = new HistoryPortService(port, (_peerId, scopeId) => {
      seen.push(scopeId);
      return spokeOk({ principal: PRINCIPAL_A, scope: SCOPE_A });
    });
    const request = {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 1,
    };
    await service.invoke("peer-a", "history.revisions.get", {
      ...request,
      scope: { scope_id: "scope_a" },
    });
    // absent scope names no selector (Rust parity: `None`).
    await service.invoke("peer-a", "history.revisions.get", request);
    expect(seen).toEqual(["scope_a", undefined]);
  });
});

describe("HistoryPortService — prune dedup", () => {
  const prunePayload = (key: string, keep = 2) => ({
    entity_kind: "knowledge_entry",
    entity_id: "ke_1",
    keep,
    idempotency_key: key,
  });

  it("same-key same-digest replay returns the recorded outcome without the port", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission());
    const first = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      prunePayload("k-1"),
    );
    expect(first).toEqual({
      ok: true,
      value: { deleted: 1, floor_revision: 2 },
    });
    const replay = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      prunePayload("k-1"),
    );
    expect(replay).toEqual({
      ok: true,
      value: { deleted: 1, floor_revision: 2 },
    });
    expect(calls).toHaveLength(1);
  });

  it("same key with a different payload digest answers IDEMPOTENCY_CONFLICT", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission());
    await service.invoke("peer-a", "history.revisions.prune", prunePayload("k-1"));
    const conflict = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      prunePayload("k-1", 5),
    );
    expect(conflict).toMatchObject({ ok: false, code: "IDEMPOTENCY_CONFLICT" });
    expect(calls).toHaveLength(1);
  });

  it("evicted recorded outcomes answer RECEIPT_EXPIRED without re-execution", async () => {
    const { port, calls } = countingPort({});
    const dedup = new InMemoryHistoryDedupStore({ maxEntries: 1 });
    const service = new HistoryPortService(port, admission(), { dedupStore: dedup });
    await service.invoke("peer-a", "history.revisions.prune", prunePayload("k-1"));
    await service.invoke("peer-a", "history.revisions.prune", prunePayload("k-2"));
    expect(calls).toHaveLength(2);
    const expired = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      prunePayload("k-1"),
    );
    expect(expired).toMatchObject({ ok: false, code: "RECEIPT_EXPIRED" });
    expect(calls).toHaveLength(2);
  });

  it("tombstones are permanent — an evicted key answers evicted forever, never unseen", async () => {
    // §9.1: capacity eviction applies to records only. Three keys at
    // maxEntries 1 evict k-1 then k-2; a bounded tombstone ring would
    // already have forgotten k-1 and replayed it as unseen.
    const dedup = new InMemoryHistoryDedupStore({ maxEntries: 1 });
    const outcome = { deleted: 1, floor_revision: 2 };
    for (const key of ["k-1", "k-2", "k-3"]) {
      await dedup.record(key, "digest", outcome);
    }
    await expect(dedup.lookup("k-1")).resolves.toEqual({ status: "evicted" });
    await expect(dedup.lookup("k-2")).resolves.toEqual({ status: "evicted" });
    await expect(dedup.lookup("k-3")).resolves.toMatchObject({
      status: "found",
    });
    await expect(dedup.lookup("never-seen")).resolves.toEqual({
      status: "unseen",
    });
  });

  it("recorded refusals replay without re-executing the port", async () => {
    let n = 0;
    const { port } = countingPort({
      prune: async () => {
        n += 1;
        return spokeReject("NOT_FOUND", "no entity");
      },
    });
    const service = new HistoryPortService(port, admission());
    await service.invoke("peer-a", "history.revisions.prune", prunePayload("k-1"));
    const replay = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      prunePayload("k-1"),
    );
    expect(replay).toMatchObject({ ok: false, code: "NOT_FOUND" });
    expect(n).toBe(1);
  });

  it("the dedup namespace binds principal + scope + op + key", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission());
    await service.invoke("peer-a", "history.revisions.prune", prunePayload("k-1"));
    await service.invoke("peer-a", "history.revisions.prune", prunePayload("k-2"));
    expect(calls).toHaveLength(2);
  });

  it("a `|` inside principal_id or idempotency_key never collides two retry identities", async () => {
    const OP = "history.revisions.prune";
    const { port, calls } = countingPort({});
    const byPeer = (peerId: string): SpokeResult<{ principal: Principal; scope: Scope }> =>
      peerId === "peer-1"
        ? spokeOk({ principal: { principal_id: "p", extensions: {} }, scope: SCOPE_A })
        : spokeOk({ principal: { principal_id: `p|${OP}|a`, extensions: {} }, scope: SCOPE_A });
    const service = new HistoryPortService(port, byPeer);
    const first = await service.invoke("peer-1", OP, prunePayload(`a|${OP}|b`));
    expect(first).toEqual({ ok: true, value: { deleted: 1, floor_revision: 2 } });
    // The delimiter-shifted identity: a distinct retry that must execute
    // its own prune — not collide with the first record.
    const second = await service.invoke("peer-2", OP, prunePayload("b"));
    expect(second).toEqual({ ok: true, value: { deleted: 1, floor_revision: 2 } });
    // An exact repeat of the second identity still replays its recorded
    // outcome without re-executing.
    const replay = await service.invoke("peer-2", OP, prunePayload("b"));
    expect(replay).toEqual(second);
    expect(calls).toHaveLength(2);
  });

  it("overlapping same-key prunes execute the port once", async () => {
    let n = 0;
    let openPort!: () => void;
    const portGate = new Promise<void>((resolve) => {
      openPort = resolve;
    });
    const { port } = countingPort({
      prune: async () => {
        n += 1;
        await portGate;
        return spokeOk({ deleted: 1, floor_revision: 2 });
      },
    });
    const service = new HistoryPortService(port, admission());
    const first = service.invoke(
      "peer-a",
      "history.revisions.prune",
      prunePayload("k-1"),
    );
    const second = service.invoke(
      "peer-a",
      "history.revisions.prune",
      prunePayload("k-1"),
    );
    openPort();
    const [firstResult, secondResult] = await Promise.all([first, second]);
    expect(firstResult).toEqual({
      ok: true,
      value: { deleted: 1, floor_revision: 2 },
    });
    expect(secondResult).toEqual({
      ok: true,
      value: { deleted: 1, floor_revision: 2 },
    });
    expect(n).toBe(1);
  });
});

describe("HistoryPortService — served success validation", () => {
  it("a wrong-target snapshot answers INTERNAL_ERROR transport, never ok", async () => {
    const { port } = countingPort({
      get: async () => spokeOk(snapshot("other_id", 9)),
    });
    const service = new HistoryPortService(port, admission());
    const result = await service.invoke("peer-a", "history.revisions.get", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 3,
    });
    expect(result).toMatchObject({
      ok: false,
      code: "INTERNAL_ERROR",
      details: { kind: "transport" },
    });
  });

  it("a malformed prune result answers INTERNAL_ERROR and the reject is the recorded outcome", async () => {
    let n = 0;
    const { port } = countingPort({
      // @ts-expect-error — a provider returning a malformed success
      prune: async () => {
        n += 1;
        return spokeOk({ deleted: 1 });
      },
    });
    const service = new HistoryPortService(port, admission());
    const payload = {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      keep: 1,
      idempotency_key: "k-1",
    };
    const result = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      payload,
    );
    expect(result).toMatchObject({
      ok: false,
      code: "INTERNAL_ERROR",
      details: { kind: "transport" },
    });
    const replay = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      payload,
    );
    expect(replay).toMatchObject({
      ok: false,
      code: "INTERNAL_ERROR",
      details: { kind: "transport" },
    });
    expect(n).toBe(1);
  });

  it("a non-positive schema_version or unknown envelope member fails the envelope", () => {
    const zero = snapshot("ke_1", 3) as unknown as Record<string, unknown>;
    zero.schema_version = 0;
    expect(isValidRevisionSnapshot(zero)).toBe(false);
    const negative = snapshot("ke_1", 3) as unknown as Record<string, unknown>;
    negative.schema_version = -1;
    expect(isValidRevisionSnapshot(negative)).toBe(false);
    const unknownMember = snapshot("ke_1", 3) as unknown as Record<
      string,
      unknown
    >;
    unknownMember.extra_member = true;
    expect(isValidRevisionSnapshot(unknownMember)).toBe(false);
  });

  it("a malformed envelope answers INTERNAL_ERROR transport on the serving face", async () => {
    const zero = snapshot("ke_1", 3) as unknown as Record<string, unknown>;
    zero.schema_version = 0;
    const { port } = countingPort({
      get: async () => spokeOk(zero as unknown as RevisionSnapshot),
    });
    const service = new HistoryPortService(port, admission());
    const result = await service.invoke("peer-a", "history.revisions.get", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 3,
    });
    expect(result).toMatchObject({
      ok: false,
      code: "INTERNAL_ERROR",
      details: { kind: "transport" },
    });
  });

  it("caller mutation of a returned prune result never changes a later replay", async () => {
    const { port } = countingPort({});
    const service = new HistoryPortService(port, admission());
    const payload = {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      keep: 1,
      idempotency_key: "k-copy",
    };
    const first = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      payload,
    );
    expect(first).toEqual({ ok: true, value: { deleted: 1, floor_revision: 2 } });
    if (first.ok) {
      const callerView = first.value as { deleted: number };
      callerView.deleted = 999;
    }
    const replay = await service.invoke(
      "peer-a",
      "history.revisions.prune",
      payload,
    );
    expect(replay).toEqual({
      ok: true,
      value: { deleted: 1, floor_revision: 2 },
    });
  });

  it("get success wraps the snapshot and preserves present-empty members", async () => {
    const withEmpty = snapshot("ke_1", 3, {
      body: { tags: [], state: {}, attributes: [] },
      modules: {},
    });
    const { port } = countingPort({
      get: async () => spokeOk(withEmpty),
    });
    const service = new HistoryPortService(port, admission());
    const result = await service.invoke("peer-a", "history.revisions.get", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 3,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      const served = result.value as { snapshot: RevisionSnapshot };
      const body = served.snapshot.snapshot as {
        body: Record<string, unknown>;
        modules?: Record<string, unknown>;
      };
      expect(body.body).toHaveProperty("tags");
      expect(body.body).toHaveProperty("state");
      expect(body.body).toHaveProperty("attributes");
      expect(body.modules).toEqual({});
      expect(JSON.parse(JSON.stringify(served.snapshot))).toEqual(
        JSON.parse(JSON.stringify(withEmpty)),
      );
    }
  });
});

describe("RevisionSnapshot — nested entity deep shape", () => {
  const malformed: Array<[string, Record<string, unknown>]> = [
    ["created_at number", { created_at: 123 }],
    ["updated_at number", { updated_at: 42 }],
    ["created_at not RFC 3339", { created_at: "July 23, 2026" }],
    ["updated_at date only", { updated_at: "2026-07-23" }],
    ["created_at offset no colon", { created_at: "2026-07-23T06:15:00+0200" }],
    ["updated_at no seconds", { updated_at: "2026-07-23T06:15Z" }],
    ["created_at bad month", { created_at: "2026-13-23T06:15:00Z" }],
    ["updated_at offset out of range", { updated_at: "2026-07-23T06:15:00+24:00" }],
    ["created_at hour 24", { created_at: "2026-07-23T24:00:00Z" }],
    [
      "extensions bad key",
      { extensions: { "Bad.Key": {} } },
    ],
    [
      "extensions scalar value",
      { extensions: { ns: 5 } },
    ],
    [
      "modules bad key",
      { modules: { "Bad.Key": {} } },
    ],
    [
      "modules scalar value",
      { modules: { activation: 3 } },
    ],
    ["owner number", { owner: 42 }],
    ["disclosure number", { disclosure: 7 }],
    ["modules array", { modules: [] }],
    ["revision string", { revision: "3" }],
    ["body.tags non-array", { body: { tags: "x" } }],
    ["body.tags non-string item", { body: { tags: [1] } }],
    ["body.state non-object", { body: { state: 5 } }],
    ["body.summary number", { body: { summary: 1 } }],
    ["body.attributes non-array", { body: { attributes: {} } }],
    [
      "body.attributes missing value",
      { body: { attributes: [{ trait_type: "t" }] } },
    ],
    [
      "body.attributes object value",
      { body: { attributes: [{ trait_type: "t", value: {} }] } },
    ],
    [
      "body.attributes bad max_value",
      { body: { attributes: [{ trait_type: "t", value: "v", max_value: "x" }] } },
    ],
    ["source_anchor non-object", { source_anchor: 1 }],
    [
      "source_anchor missing source_id",
      { source_anchor: { schema_version: 1, extensions: {} } },
    ],
    [
      "source_anchor bad span",
      {
        source_anchor: {
          schema_version: 1,
          source_id: "s1",
          span: { start: "0", end: 4 },
          extensions: {},
        },
      },
    ],
  ];

  it.each(malformed)(
    "knowledge snapshot with %s fails isValidRevisionSnapshot",
    (_name, extras) => {
      expect(isValidRevisionSnapshot(snapshot("ke_1", 3, extras))).toBe(false);
    },
  );

  it.each([
    ["created_at number", { created_at: 123 }],
    ["label number", { label: 42 }],
    ["metadata array", { metadata: [] }],
    ["revision string", { revision: "3" }],
  ])(
    "relation snapshot with %s fails isValidRevisionSnapshot",
    (_name, extras) => {
      const relation = snapshot("rel_1", 2);
      const entity = relation.snapshot as unknown as Record<string, unknown>;
      relation.entity_kind = "relation";
      delete entity.entry_id;
      delete entity.entry_type;
      delete entity.canonical_name;
      delete entity.status;
      delete entity.body;
      Object.assign(entity, {
        relation_id: "rel_1",
        relation_type: "located_in",
        from_id: "ke_a",
        to_id: "ke_b",
        ...extras,
      });
      expect(isValidRevisionSnapshot(relation)).toBe(false);
    },
  );

  it("a malformed nested member answers INTERNAL_ERROR on the serving face", async () => {
    const { port } = countingPort({
      get: async () => spokeOk(snapshot("ke_1", 3, { created_at: 123 })),
    });
    const service = new HistoryPortService(port, admission());
    const result = await service.invoke("peer-a", "history.revisions.get", {
      entity_kind: "knowledge_entry",
      entity_id: "ke_1",
      revision: 3,
    });
    expect(result).toMatchObject({
      ok: false,
      code: "INTERNAL_ERROR",
      details: { kind: "transport" },
    });
  });

  it("a snapshot with declared optional members still decodes", () => {
    const valid = snapshot("ke_1", 3, {
      created_at: "2026-07-23T06:15:00Z",
      updated_at: "2026-07-23T06:15:00Z",
      owner: "ke_owner",
      disclosure: "owner-private",
      modules: { activation: { level: 1 } },
      body: {
        summary: "s",
        tags: ["t1"],
        attributes: [{ trait_type: "t", value: 4, display_type: "n" }],
        state: { mood: "calm" },
        computable: { score: 0.5 },
      },
      source_anchor: {
        schema_version: 1,
        source_id: "src_1",
        span: { start: 0, end: 4 },
        label: "l",
        mime_type: "text/plain",
        extensions: {},
      },
    });
    expect(isValidRevisionSnapshot(valid)).toBe(true);
    const relation = snapshot("rel_1", 2);
    const entity = relation.snapshot as unknown as Record<string, unknown>;
    relation.entity_kind = "relation";
    for (const key of [
      "entry_id",
      "entry_type",
      "canonical_name",
      "status",
      "body",
    ]) {
      delete entity[key];
    }
    Object.assign(entity, {
      relation_id: "rel_1",
      relation_type: "located_in",
      from_id: "ke_a",
      to_id: "ke_b",
      label: "l",
      metadata: { any: true },
      created_at: "2026-07-23T06:15:00Z",
      updated_at: "2026-07-23T06:15:00Z",
    });
    expect(isValidRevisionSnapshot(relation)).toBe(true);
  });
  it("a leap-second timestamp still decodes (chrono parity)", () => {
    const leap = snapshot("ke_1", 3, {
      created_at: "2026-07-23T23:59:60Z",
      updated_at: "2026-07-23T23:59:60.5Z",
    });
    expect(isValidRevisionSnapshot(leap)).toBe(true);
  });
  it("an invalid calendar day is rejected even with a leap-second clamp", () => {
    const invalidDay = snapshot("ke_1", 3, {
      created_at: "2026-02-30T23:59:60Z",
      updated_at: "2026-02-30T23:59:60Z",
    });
    expect(isValidRevisionSnapshot(invalidDay)).toBe(false);
  });

});

// ——— loopback responder ↔ RemoteAdapter ———

interface DialedHistoryHost {
  client: RemoteAdapter;
  close: () => void;
}

async function dialHistoryHost(options: {
  ports: Parameters<typeof connectResponder>[0]["ports"];
  clientCapabilities: string[];
  hostCapabilities?: string[];
  hostInvokeTimeoutMs?: number;
}): Promise<DialedHistoryHost> {
  const hostSeed = seed(0xa0);
  const hostPubkey = getPublicKeyEd25519(hostSeed);
  const hostPeerId = derivePeerIdFromEd25519Pubkey(hostPubkey);
  const clientSeed = seed(0xb0);
  const clientPubkey = getPublicKeyEd25519(clientSeed);
  const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
  const pair = loopbackTransportPair();
  const host = await connectResponder({
    transport: pair.server,
    identity: { seed: hostSeed },
    manifest: manifest("history-host", options.hostCapabilities ?? ["spoke-history"]),
    allowlist: [clientPeerId],
    peerKeys: { [clientPeerId]: clientPubkey },
    ports: options.ports,
    invokeTimeoutMs: options.hostInvokeTimeoutMs,
  });
  const client = await connectRemoteAdapter({
    transport: pair.client,
    localIdentity: { seed: clientSeed },
    localManifest: manifest("client", options.clientCapabilities),
    remotePubkey: hostPubkey,
    allowlist: [hostPeerId],
  });
  return {
    client,
    close: (): void => {
      client.close();
      host.close();
    },
  };
}

describe("history serving over the loopback responder", () => {
  it("unnegotiated spoke-history answers the capability gate, never the port", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission());
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-baseline"],
      ports: { asHistory: () => service },
    });
    try {
      const result = await host.client.getHistorySnapshot(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        1,
      );
      expect(result).toMatchObject({
        ok: false,
        code: "CAPABILITY_PORT_MISSING",
        details: { wire_code: "capability_missing" },
      });
      expect(calls).toHaveLength(0);
    } finally {
      host.close();
    }
  });

  it("absent face answers the dispatch-deny branch", async () => {
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: {},
    });
    try {
      const result = await host.client.getHistorySnapshot(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        1,
      );
      expect(result).toMatchObject({
        ok: false,
        code: "CAPABILITY_PORT_MISSING",
        details: { wire_code: "op_unsupported" },
      });
    } finally {
      host.close();
    }
  });

  it("a malformed request answers INVALID_INPUT before the port", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission());
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
    });
    try {
      const result = await host.client.pruneHistory(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        2,
        { idempotencyKey: "" },
      );
      expect(result).toMatchObject({ ok: false, code: "INVALID_INPUT" });
      expect(calls).toHaveLength(0);
    } finally {
      host.close();
    }
  });

  it("get replays a snapshot with present-empty members byte-identically", async () => {
    const served = snapshot("ke_1", 3, {
      body: { tags: [], state: {}, attributes: [] },
      modules: {},
    });
    const { port } = countingPort({
      get: async () => spokeOk(served),
    });
    const service = new HistoryPortService(port, admission());
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
    });
    try {
      const result = await host.client.getHistorySnapshot(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        3,
      );
      expect(result.ok).toBe(true);
      if (result.ok) {
        expect(result.value).toEqual(served);
        const body = result.value.snapshot as {
          body: Record<string, unknown>;
          modules?: Record<string, unknown>;
        };
        expect("tags" in body.body && "state" in body.body).toBe(true);
        expect(body.modules).toEqual({});
      }
    } finally {
      host.close();
    }
  });

  it("a caller-supplied idempotency key dedups the prune replay", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission());
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
    });
    try {
      const first = await host.client.pruneHistory(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        2,
        { idempotencyKey: "retry-key-1" },
      );
      expect(first).toEqual({
        ok: true,
        value: { deleted: 1, floor_revision: 2 },
      });
      const replay = await host.client.pruneHistory(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        2,
        { idempotencyKey: "retry-key-1" },
      );
      expect(replay).toEqual({
        ok: true,
        value: { deleted: 1, floor_revision: 2 },
      });
      expect(calls).toHaveLength(1);
    } finally {
      host.close();
    }
  });
  it("a same-key different-digest replay answers IDEMPOTENCY_CONFLICT over the wire", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission());
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
    });
    try {
      await host.client.pruneHistory(PRINCIPAL_A, SCOPE_A, "knowledge_entry", "ke_1", 2, {
        idempotencyKey: "conflict-key",
      });
      const conflict = await host.client.pruneHistory(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        5,
        { idempotencyKey: "conflict-key" },
      );
      expect(conflict).toMatchObject({ ok: false, code: "IDEMPOTENCY_CONFLICT" });
      expect(calls).toHaveLength(1);
    } finally {
      host.close();
    }
  });

  it("a replay against an evicted recorded outcome answers RECEIPT_EXPIRED", async () => {
    const { port, calls } = countingPort({});
    const service = new HistoryPortService(port, admission(), {
      dedupStore: new InMemoryHistoryDedupStore({ maxEntries: 1 }),
    });
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
    });
    try {
      await host.client.pruneHistory(PRINCIPAL_A, SCOPE_A, "knowledge_entry", "ke_1", 1, {
        idempotencyKey: "evicted-key",
      });
      await host.client.pruneHistory(PRINCIPAL_A, SCOPE_A, "knowledge_entry", "ke_1", 1, {
        idempotencyKey: "evictor-key",
      });
      const expired = await host.client.pruneHistory(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        1,
        { idempotencyKey: "evicted-key" },
      );
      expect(expired).toMatchObject({ ok: false, code: "RECEIPT_EXPIRED" });
      expect(calls).toHaveLength(2);
    } finally {
      host.close();
    }
  });

  it("a wrong-target provider snapshot is a transport decode error, never ok", async () => {
    const { port } = countingPort({
      get: async () => spokeOk(snapshot("ke_other", 9)),
    });
    const service = new HistoryPortService(port, admission());
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
    });
    try {
      const result = await host.client.getHistorySnapshot(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        3,
      );
      expect(result).toMatchObject({
        ok: false,
        code: "INTERNAL_ERROR",
        details: { kind: "transport" },
      });
    } finally {
      host.close();
    }
  });

  it("a malformed snapshot envelope is a transport decode error at the RemoteAdapter boundary", async () => {
    const zero = snapshot("ke_1", 3) as unknown as Record<string, unknown>;
    zero.schema_version = 0;
    const rawService: RemoteHistoryService = {
      invoke: async () => spokeOk({ snapshot: zero }),
    };
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => rawService },
    });
    try {
      const result = await host.client.getHistorySnapshot(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        3,
      );
      expect(result).toMatchObject({
        ok: false,
        code: "INTERNAL_ERROR",
        details: { kind: "transport" },
      });
    } finally {
      host.close();
    }
  });

  it("a read serve budget expiring mid-invoke answers the plain serve timeout", async () => {
    const { port, calls } = countingPort({
      get: async (_principal, _scope, _kind, id, revision) => {
        calls.push(`get:${id}@${revision}`);
        await new Promise<void>((resolve) => setTimeout(resolve, 200));
        return spokeOk(snapshot(id, revision));
      },
    });
    const service = new HistoryPortService(port, admission());
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
      hostInvokeTimeoutMs: 60,
    });
    try {
      const timedOut = await host.client.getHistorySnapshot(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        3,
      );
      expect(timedOut.ok).toBe(false);
      if (!timedOut.ok) {
        // A read cannot have committed: plain internal error, no
        // `retry_class` (exact details equality proves its absence).
        expect(timedOut.code).toBe("INTERNAL_ERROR");
        expect(timedOut.details).toEqual({ kind: "timeout" });
      }
      expect(calls).toEqual(["get:ke_1@3"]);
    } finally {
      host.close();
    }
  });

  it("a throwing history read answers the plain internal error with no details", async () => {
    const service: RemoteHistoryService = {
      invoke: async (_peerId, op) => {
        if (op !== "history.revisions.get") {
          throw new Error(`unexpected op ${op}`);
        }
        throw new Error("history get exploded");
      },
    };
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
    });
    try {
      const failed = await host.client.getHistorySnapshot(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        3,
      );
      expect(failed.ok).toBe(false);
      if (!failed.ok) {
        // A read cannot have committed: plain internal error, no `details`
        // and therefore no `retry_class`.
        expect(failed.code).toBe("INTERNAL_ERROR");
        expect(failed.message).toBe(
          "history service failed after invocation: history get exploded",
        );
        expect(failed.details).toBeUndefined();
      }
    } finally {
      host.close();
    }
  });

  it("a throwing history prune stays uncertain with the panic class", async () => {
    const service: RemoteHistoryService = {
      invoke: async (_peerId, op) => {
        if (op !== "history.revisions.prune") {
          throw new Error(`unexpected op ${op}`);
        }
        throw new Error("history prune exploded");
      },
    };
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
    });
    try {
      const failed = await host.client.pruneHistory(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        2,
        { idempotencyKey: "throwing-prune" },
      );
      expect(failed.ok).toBe(false);
      if (!failed.ok) {
        expect(failed.code).toBe("OUTCOME_UNCERTAIN");
        expect(failed.details).toEqual({
          retry_class: "uncertain",
          kind: "panic",
        });
      }
    } finally {
      host.close();
    }
  });

  it("a serve budget expiring mid-invoke answers OUTCOME_UNCERTAIN and a same-key replay recovers", async () => {
    const { port, calls } = countingPort({
      prune: async (_principal, _scope, _kind, id, keep) => {
        calls.push(`prune:${id}@${keep}`);
        await new Promise<void>((resolve) => setTimeout(resolve, 200));
        return spokeOk({ deleted: 1, floor_revision: 2 });
      },
    });
    // The observable "the detached invoke settled" is the dedup record
    // being written — wait for that, not a duration.
    const dedup = new InMemoryHistoryDedupStore();
    const recorded = new Promise<void>((resolve) => {
      const original = dedup.record.bind(dedup);
      dedup.record = async (key, digest, outcome) => {
        await original(key, digest, outcome);
        resolve();
      };
    });
    const service = new HistoryPortService(port, admission(), { dedupStore: dedup });
    const host = await dialHistoryHost({
      clientCapabilities: ["spoke-history"],
      ports: { asHistory: () => service },
      hostInvokeTimeoutMs: 60,
    });
    try {
      const timedOut = await host.client.pruneHistory(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        2,
        { idempotencyKey: "uncertain-key" },
      );
      expect(timedOut).toMatchObject({
        ok: false,
        code: "OUTCOME_UNCERTAIN",
        details: { retry_class: "uncertain", kind: "timeout" },
      });
      // The detached invoke keeps running: it settles by recording.
      await recorded;
      const replay = await host.client.pruneHistory(
        PRINCIPAL_A,
        SCOPE_A,
        "knowledge_entry",
        "ke_1",
        2,
        { idempotencyKey: "uncertain-key" },
      );
      expect(replay).toEqual({
        ok: true,
        value: { deleted: 1, floor_revision: 2 },
      });
      expect(calls).toHaveLength(1);
    } finally {
      host.close();
    }
  });
});

// ——— canonical corpus vectors over the same loopback ———

describe("history-conformance canonical vectors (JCS parity)", () => {
  const corpus = JSON.parse(
    readFileSync(
      new URL(
        "../../../../fixtures/history-conformance/canonical-snapshot.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ) as {
    cases: Array<{
      id: string;
      input: {
        entities: Array<{
          entity_kind: string;
          entity_id: string;
          stored_snapshot: RevisionSnapshot;
        }>;
      };
      request: { entity_kind: string; entity_id: string; revision: number };
    }>;
  };

  it("every canonical vector replays to RFC 8785-identical bytes over the wire", async () => {
    for (const vector of corpus.cases) {
      const entity = vector.input.entities[0];
      const { port } = countingPort({
        get: async () => spokeOk(entity.stored_snapshot),
      });
      const service = new HistoryPortService(port, admission());
      const host = await dialHistoryHost({
        clientCapabilities: ["spoke-history"],
        ports: { asHistory: () => service },
      });
      try {
        const result = await host.client.getHistorySnapshot(
          PRINCIPAL_A,
          SCOPE_A,
          vector.request.entity_kind,
          vector.request.entity_id,
          vector.request.revision,
        );
        expect(result.ok, `case ${vector.id}`).toBe(true);
        if (result.ok) {
          // Contract §RevisionSnapshot: two providers returning the same
          // snapshot at the same revision produce identical canonical bytes.
          expect(canonicalize(result.value)).toBe(
            canonicalize(entity.stored_snapshot),
          );
        }
      } finally {
        host.close();
      }
    }
  });
});
