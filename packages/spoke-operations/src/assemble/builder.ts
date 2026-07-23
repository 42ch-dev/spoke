import type {
  AssemblePacket,
  ExtensionMap,
  KnowledgeEntry,
} from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

type AssembleEntry = AssemblePacket["entries"][number];

const DEFAULT_SCHEMA_VERSION = 1;

function extractSnippet(knowledgeEntry: KnowledgeEntry): string | undefined {
  const { body } = knowledgeEntry;

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

function validateAssembleKnowledgeEntry(
  knowledgeEntry: KnowledgeEntry,
): SpokeResult<void> {
  if (
    typeof knowledgeEntry.body !== "object" ||
    knowledgeEntry.body === null ||
    Array.isArray(knowledgeEntry.body)
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_PACKET_INPUT,
      "KnowledgeEntry body must be an object",
      { knowledge_entry_id: knowledgeEntry.knowledge_entry_id, field: "body" },
    );
  }

  if (typeof knowledgeEntry.canonical_name !== "string") {
    return spokeReject(
      SpokeRejectCode.INVALID_PACKET_INPUT,
      "KnowledgeEntry canonical_name must be a string",
      {
        knowledge_entry_id: knowledgeEntry.knowledge_entry_id,
        field: "canonical_name",
      },
    );
  }

  if (knowledgeEntry.canonical_name.trim().length === 0) {
    return spokeReject(
      SpokeRejectCode.INVALID_PACKET_INPUT,
      "KnowledgeEntry canonical_name must be non-empty",
      {
        knowledge_entry_id: knowledgeEntry.knowledge_entry_id,
        field: "canonical_name",
      },
    );
  }

  return spokeOk();
}

/**
 * Map a KnowledgeEntry to a slim AssembleEntry per wire rules.
 */
export function knowledgeEntryToAssembleEntry(
  knowledgeEntry: KnowledgeEntry,
): AssembleEntry {
  const entry: AssembleEntry = {
    knowledge_entry_id: knowledgeEntry.knowledge_entry_id,
    entry_type: knowledgeEntry.entry_type,
    canonical_name: knowledgeEntry.canonical_name,
  };

  const snippet = extractSnippet(knowledgeEntry);
  if (snippet !== undefined) {
    entry.snippet = snippet;
  }

  return entry;
}

export type BuildAssemblePacketInput = {
  packetId: string;
  knowledgeEntries: KnowledgeEntry[];
  extensions?: ExtensionMap;
  maxEntries?: number;
};

/**
 * Build a wire-valid AssemblePacket from KnowledgeEntries (order-preserving truncate only).
 */
export function buildAssemblePacket(
  input: BuildAssemblePacketInput,
): SpokeResult<AssemblePacket> {
  const { packetId, knowledgeEntries, extensions, maxEntries } = input;

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

  for (const knowledgeEntry of knowledgeEntries) {
    const knowledgeEntryResult = validateAssembleKnowledgeEntry(knowledgeEntry);
    if (!knowledgeEntryResult.ok) {
      return knowledgeEntryResult;
    }
  }

  const entries = knowledgeEntries.map(knowledgeEntryToAssembleEntry);
  const truncatedEntries =
    maxEntries === undefined ? entries : entries.slice(0, maxEntries);

  return spokeOk({
    schema_version: DEFAULT_SCHEMA_VERSION,
    packet_id: packetId,
    entries: truncatedEntries,
    extensions: extensions ?? {},
  });
}
