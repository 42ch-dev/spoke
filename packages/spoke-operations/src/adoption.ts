/**
 * `spoke-knowledge-adoption` family helpers: the `adoption_anchor`
 * immutability gate for the normal put path, the bounded retirement-chain
 * resolver, and the destination copy builder (spoke-knowledge-adoption
 * contract §Anchor immutability / §Retirement-chain resolution /
 * §Copy rules and create-only OCC).
 *
 * Equality of two anchors is RFC 8785 canonical-JSON byte equality — the
 * same rule `history.ts` `revisionSnapshotsEqualCanonical` applies, so a
 * put that carries the identical stored anchor never refuses.
 *
 * Rust parity: `crates/spoke-operations/src/adoption.rs`.
 */

import canonicalize from "canonicalize";

import type { KnowledgeEntry } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "./result.js";

/** Retirement edges followed after the requested entry before SOURCE_RETIRED. */
export const RETIREMENT_CHAIN_MAX_EDGES = 32;

/**
 * Legacy product-private adoption encoding namespace (the superseded
 * mindcore `extensions.mindcore_adoption` codec): dropped verbatim from the
 * destination copy; every other namespace and key survives unchanged,
 * including explicitly empty namespace objects.
 */
export const ADOPTION_LEGACY_EXTENSIONS_NAMESPACE = "mindcore_adoption";

/**
 * RFC 8785 canonical JSON bytes (as a string) of `value`, or `null` when the
 * value cannot be canonicalized (mirrors `serde_jcs::to_vec` failure).
 */
function canonicalJson(value: unknown): string | undefined {
  try {
    return canonicalize(value);
  } catch {
    return undefined;
  }
}

/**
 * The `adoption_anchor` immutability gate for a normal put
 * (spoke-knowledge-adoption contract §Anchor immutability): once the
 * stored entry carries an `adoption_anchor`, a put must preserve it
 * unchanged. A candidate that replaces it with a different value or
 * removes it refuses `ADOPTION_ANCHOR_IMMUTABLE` with `details.entry_id`
 * naming the destination entry; a candidate carrying the identical anchor
 * (canonical-byte equal) or no anchor when the stored entry has none
 * passes.
 */
export function assertAdoptionAnchorPreserved(
  candidate: KnowledgeEntry,
  stored: KnowledgeEntry | undefined,
): SpokeResult<void> {
  const storedAnchor = stored?.adoption_anchor;
  if (storedAnchor === undefined) {
    return spokeOk();
  }
  const candidateAnchor = candidate.adoption_anchor;
  if (
    candidateAnchor !== undefined &&
    canonicalJson(candidateAnchor) === canonicalJson(storedAnchor)
  ) {
    return spokeOk();
  }
  return spokeReject(
    "ADOPTION_ANCHOR_IMMUTABLE",
    "a normal put must preserve the stored adoption_anchor unchanged",
    { entry_id: stored?.entry_id ?? candidate.entry_id },
  );
}

/**
 * One visited retirement-chain node: the entry id plus the terminal-entry
 * fields the resolver needs (`status`, and `scope_id` for the recorded
 * terminal scope). Successor traversal is delegated to the provider —
 * how a `merged` entry names its successors is storage-owned; the helper
 * owns only the chain's termination semantics.
 */
export interface RetirementChainEntry {
  /** Entry id as visited (the requested id first). */
  entry_id: string;
  /** Scope id the entry was stored under. */
  scope_id: string;
  /** Status vocabulary (`merged` follows edges, `deleted` is terminal refusal). */
  status: string;
  /** Revision of this entry (recorded on the anchor for the terminal). */
  revision?: number;
}

/**
 * Storage-owned successor lookup for one `merged` chain node: the entry
 * ids this node merged into (or the successor nodes themselves via the
 * loader contract). Returning `undefined` means the provider could not
 * load or identify successors — treated as an ambiguous chain. An empty
 * array means no successor; more than one means an ambiguous merge.
 */
export type RetirementSuccessorsOf = (
  entry: RetirementChainEntry,
) => Promise<readonly string[] | undefined>;

/**
 * Storage-owned chain-node loader: the entry stored under `entry_id` in
 * the source scope, or `undefined` when absent. A mid-chain miss refuses
 * `SOURCE_RETIRED` (the chain cannot reach a live terminal); the
 * requested entry's own absence is caller-owned (`NOT_FOUND` before the
 * resolver runs).
 */
export type RetirementLoadEntry = (
  entryId: string,
) => Promise<RetirementChainEntry | undefined>;

/** Successful resolution: the terminal live source the anchor records. */
export interface RetirementTerminal {
  /** Resolved terminal source scope id. */
  source_scope_id: string;
  /** Resolved terminal source entry id. */
  source_entry_id: string;
  /** Resolved terminal source revision (the node's own `revision`, or 0). */
  source_revision: number;
}

/**
 * Resolve a requested source's retirement chain to the terminal live
 * entry (spoke-knowledge-adoption contract §Retirement-chain resolution).
 *
 * `requested` is the already-loaded, already-admitted requested entry (a
 * missing or non-disclosable source refuses `NOT_FOUND` before this
 * helper runs — the resolver never sees it). A live requested source
 * resolves to itself without touching the chain. From a `merged` node
 * resolution follows at most `RETIREMENT_CHAIN_MAX_EDGES` (32) edges;
 * every visited entry id is tracked, and `details.retirement_chain` on
 * `SOURCE_RETIRED` is the ordered visited-id array beginning with the
 * requested id and including the entry at which resolution failed.
 * Refusals: a repeated visited id (cycle), zero or multiple successors on
 * a `merged` node (ambiguous chain), a `deleted` terminal, a mid-chain
 * load miss, or a required 33rd edge.
 */
export async function resolveRetirementChain(
  requested: RetirementChainEntry,
  successorsOf: RetirementSuccessorsOf,
  loadEntry: RetirementLoadEntry,
): Promise<SpokeResult<RetirementTerminal>> {
  const visited: string[] = [requested.entry_id];
  const seen = new Set(visited);
  let current = requested;
  while (current.status === "merged") {
    const successors = await successorsOf(current);
    if (successors === undefined || successors.length !== 1) {
      return spokeReject(
        "SOURCE_RETIRED",
        "retirement chain is ambiguous: a merged entry has zero or multiple successors",
        { retirement_chain: visited },
      );
    }
    const [successorId] = successors;
    if (visited.length > RETIREMENT_CHAIN_MAX_EDGES) {
      // note: reaching the 33rd edge means 32 edges were already followed
      // and the chain still has not terminated. `retirement_chain` lists
      // visited ids only — the successor beyond the rejected edge was
      // never loaded, so it stays unlisted (contract §Retirement-chain
      // resolution).
      return spokeReject(
        "SOURCE_RETIRED",
        "retirement chain exceeds the 32-edge limit",
        { retirement_chain: visited },
      );
    }
    if (seen.has(successorId)) {
      visited.push(successorId);
      return spokeReject(
        "SOURCE_RETIRED",
        "retirement chain cycle: a visited entry id repeats",
        { retirement_chain: visited },
      );
    }
    const successor = await loadEntry(successorId);
    if (successor === undefined) {
      // note: `retirement_chain` ends at the last resolved node — the
      // unresolvable successor stays unlisted (contract §Retirement-chain
      // resolution).
      return spokeReject(
        "SOURCE_RETIRED",
        "retirement chain is broken: the successor entry is missing",
        { retirement_chain: visited },
      );
    }
    visited.push(successorId);
    seen.add(successorId);
    current = successor;
  }
  if (current.status === "deleted") {
    return spokeReject(
      "SOURCE_RETIRED",
      "retirement chain terminal entry is deleted",
      { retirement_chain: visited },
    );
  }
  return spokeOk({
    source_scope_id: current.scope_id,
    source_entry_id: current.entry_id,
    source_revision: current.revision ?? 0,
  });
}

