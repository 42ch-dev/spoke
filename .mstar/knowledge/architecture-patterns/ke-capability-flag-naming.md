---
module: capability flags / carrier anchoring
date: 2026-09-17
last_updated: 2026-11-07
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
tags:
  - capability-flag
  - naming
  - carrier-anchored
  - cognition
  - ke-ownership
  - ke-extraction
  - negotiation
---

# Capability flag naming — anchor the flag to its carrier

## Context

Spoke capability flags exist in three shapes: layer-anchored `l<N>-<concern>` (`l2-computable`, `l5-fork`), named cross-layer families (`ke-ownership`, `ke-extraction`, `narrative-modules`, `spoke-connect`), and the `spoke-cognition` family. The layer-prefixed form is reflexively proposed for each new concern and misdescribes any concern whose carrier spans layers.

## Guidance

Name a capability flag after the concern's **actual carrier**:

- Use `l<N>-<concern>` **only** when the concern is genuinely anchored to one existing protocol layer (`fork` → L5 timeline; `computable` → L2 body).
- When the concern spans layers, use a **named family** that names the carrier: `ke-<concern>` for KE governance/extraction, `spoke-cognition` for the cognition family.
- One concern gets one flag. Never an umbrella flag over independently releasable siblings, and independent flags carry no prerequisite ordering (`ke-ownership` and `ke-extraction` are independent).

Frozen rulings that travel with these flags:

- **Viewpoint selection lives on the shared `Scope`** — `Scope.cognition: CognitionSelection`, one optional field consumed by check/assemble through the existing `$ref`; no request-only sibling field.
- **`spoke-cognition` is one family with no subflags.** It covers the three record categories on `KnowledgeEntry.modules.cognition`, the Scope selection, the assemble projection and the Memory `Content.cognition` / `cognition_filter` surfaces. A host advertises it only when every mandatory rule for its actually-served ports holds: a KE-only host conforms without Memory ports, and a host serving both must satisfy the Memory rules before advertising the combined surface.
- **No layer-anchored flag implies `spoke-cognition`.** The signed-hello intersection across differing flag names is empty, so a request that requires the family on a host which does not declare `spoke-cognition` takes the existing `capability_missing` branch with no handler side effect.
- **`owner` references a holder KE `entry_id`** (no reserved `world` token, no subject registry, no closed union); `owner-private` is documented vocabulary, not an enum.

## Why This Matters

A layer prefix implies a normative layer definition that does not describe the concern; inventing `l1-ownership` or a layer-prefixed cognition flag creates a false taxonomy every later flag must fight. A layer-anchored flag read as an alias is worse: it opens a silent second negotiation path, so an old host would appear to support a family it does not implement. Carrier-anchored names keep the flag table honest and capability negotiation legible.

## When to Apply

Any new capability flag. Check the concern against the existing layer definitions first; anchor to a layer only when that layer owns the whole concern. When a flag spans surfaces, enumerate the served ports whose conformance gates advertisement.

## Examples

- `ke-ownership` / `ke-extraction` — [`.mstar/specs/ke-ownership-disclosure-adr.md`](../../specs/ke-ownership-disclosure-adr.md) §1.1, [`.mstar/specs/ke-extraction-adr.md`](../../specs/ke-extraction-adr.md) §1.1.
- `spoke-cognition` — one family over KE modules, Scope selection, assemble projection and Memory content/filters; conformance per actually-served port ([`.mstar/specs/spoke-cognition-consumer-contract.md`](../../specs/spoke-cognition-consumer-contract.md)), with negotiation and a payload-dependent guard on the existing ops.
- Related: [`mind-axis-ownership-boundary.md`](mind-axis-ownership-boundary.md) (dual-SSOT prevention), [`capability-flagged-optional-bag.md`](capability-flagged-optional-bag.md) (shipping pattern), [`spoke-connect-wire-and-auth.md`](spoke-connect-wire-and-auth.md) (existing negotiation and dispatch gate).
