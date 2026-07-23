import type {
  AssemblePacket,
  ExtensionMap,
  Keyblock,
} from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

type AssembleEntry = AssemblePacket["entries"][number];

const DEFAULT_SCHEMA_VERSION = 1;

function extractSnippet(keyblock: Keyblock): string | undefined {
  const { body } = keyblock;

  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return undefined;
  }

  const summary = body.summary;

  if (typeof summary !== "string") {
    return undefined;
  }

  const trimmed = summary.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

function validateAssembleKeyblock(keyblock: Keyblock): SpokeResult<void> {
  if (typeof keyblock.body !== "object" || keyblock.body === null || Array.isArray(keyblock.body)) {
    return spokeReject(
      SpokeRejectCode.INVALID_PACKET_INPUT,
      "Keyblock body must be an object",
      { keyblock_id: keyblock.keyblock_id, field: "body" },
    );
  }

  if (typeof keyblock.canonical_name !== "string") {
    return spokeReject(
      SpokeRejectCode.INVALID_PACKET_INPUT,
      "Keyblock canonical_name must be a string",
      { keyblock_id: keyblock.keyblock_id, field: "canonical_name" },
    );
  }

  if (keyblock.canonical_name.trim().length === 0) {
    return spokeReject(
      SpokeRejectCode.INVALID_PACKET_INPUT,
      "Keyblock canonical_name must be non-empty",
      { keyblock_id: keyblock.keyblock_id, field: "canonical_name" },
    );
  }

  return spokeOk();
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

  for (const keyblock of keyblocks) {
    const keyblockResult = validateAssembleKeyblock(keyblock);
    if (!keyblockResult.ok) {
      return keyblockResult;
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
