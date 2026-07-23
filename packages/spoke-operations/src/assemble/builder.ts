import type {
  AssemblePacket,
  ExtensionMap,
  Keyblock,
} from "@42ch/spoke-schema";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

type AssembleEntry = AssemblePacket["entries"][number];

const DEFAULT_SCHEMA_VERSION = 1;

function extractSnippet(keyblock: Keyblock): string | undefined {
  const summary = keyblock.body.summary;

  if (typeof summary !== "string") {
    return undefined;
  }

  const trimmed = summary.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

/**
 * Map a Keyblock to a slim AssembleEntry per wire rules.
 */
export function keyblockToAssembleEntry(keyblock: Keyblock): AssembleEntry {
  const entry: AssembleEntry = {
    keyblock_id: keyblock.keyblock_id,
    block_type: keyblock.block_type,
    canonical_name: keyblock.canonical_name,
  };

  const snippet = extractSnippet(keyblock);
  if (snippet !== undefined) {
    entry.snippet = snippet;
  }

  return entry;
}

export type BuildAssemblePacketInput = {
  packetId: string;
  keyblocks: Keyblock[];
  extensions?: ExtensionMap;
  maxEntries?: number;
};

/**
 * Build a wire-valid AssemblePacket from Keyblocks (order-preserving truncate only).
 */
export function buildAssemblePacket(
  input: BuildAssemblePacketInput,
): SpokeResult<AssemblePacket> {
  const { packetId, keyblocks, extensions, maxEntries } = input;

  if (typeof packetId !== "string" || packetId.trim().length === 0) {
    return spokeReject(
      SpokeRejectCode.INVALID_PACKET_INPUT,
      "packetId must be a non-empty string",
      { packetId },
    );
  }

  if (maxEntries !== undefined) {
    if (!Number.isInteger(maxEntries) || maxEntries < 0) {
      return spokeReject(
        SpokeRejectCode.INVALID_PACKET_INPUT,
        "maxEntries must be a non-negative integer",
        { maxEntries },
      );
    }
  }

  const entries = keyblocks.map(keyblockToAssembleEntry);
  const truncatedEntries =
    maxEntries === undefined ? entries : entries.slice(0, maxEntries);

  return spokeOk({
    schema_version: DEFAULT_SCHEMA_VERSION,
    packet_id: packetId,
    entries: truncatedEntries,
    extensions: extensions ?? {},
  });
}
