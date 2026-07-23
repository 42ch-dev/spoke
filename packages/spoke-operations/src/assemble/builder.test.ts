import type { Keyblock } from "@42ch/spoke-schema";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  buildAssemblePacket,
  keyblockToAssembleEntry,
} from "./builder.js";

function makeKeyblock(overrides: Partial<Keyblock> = {}): Keyblock {
  return {
    schema_version: 1,
    keyblock_id: "kb_1",
    block_type: "character",
    canonical_name: "Mira Vale",
    status: "confirmed",
    body: {},
    extensions: {},
    ...overrides,
  };
}

describe("keyblockToAssembleEntry", () => {
  it("maps core fields", () => {
    const entry = keyblockToAssembleEntry(
      makeKeyblock({ body: { summary: "  Hero  " } }),
    );

    expect(entry).toEqual({
      keyblock_id: "kb_1",
      block_type: "character",
      canonical_name: "Mira Vale",
      snippet: "Hero",
    });
  });

  it("omits snippet for non-string body.summary", () => {
    const entry = keyblockToAssembleEntry(
      makeKeyblock({ body: { summary: 42 } }),
    );

    expect(entry).not.toHaveProperty("snippet");
  });

  it("omits snippet for whitespace-only summary", () => {
    const entry = keyblockToAssembleEntry(
      makeKeyblock({ body: { summary: "   " } }),
    );

    expect(entry).not.toHaveProperty("snippet");
  });
});

describe("buildAssemblePacket", () => {
  it("builds packet with empty keyblock list", () => {
    const result = buildAssemblePacket({ packetId: "pkt_1", keyblocks: [] });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual({
        schema_version: 1,
        packet_id: "pkt_1",
        entries: [],
        extensions: {},
      });
    }
  });

  it("truncates entries in input order with maxEntries", () => {
    const keyblocks = [
      makeKeyblock({ keyblock_id: "kb_a", canonical_name: "A" }),
      makeKeyblock({ keyblock_id: "kb_b", canonical_name: "B" }),
      makeKeyblock({ keyblock_id: "kb_c", canonical_name: "C" }),
    ];

    const result = buildAssemblePacket({
      packetId: "pkt_2",
      keyblocks,
      maxEntries: 2,
    });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.entries.map((entry) => entry.keyblock_id)).toEqual([
        "kb_a",
        "kb_b",
      ]);
    }
  });

  it("passes extensions through", () => {
    const extensions = { nexus: { profile: "chat" } };
    const result = buildAssemblePacket({
      packetId: "pkt_3",
      keyblocks: [],
      extensions,
    });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.extensions).toEqual(extensions);
    }
  });

  it("rejects empty packetId", () => {
    const result = buildAssemblePacket({ packetId: "  ", keyblocks: [] });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_PACKET_INPUT);
    }
  });

  it("rejects negative maxEntries", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_4",
      keyblocks: [],
      maxEntries: -1,
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_PACKET_INPUT);
    }
  });
});
