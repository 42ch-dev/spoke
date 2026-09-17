---
module: capability flags / KE governance axis
date: 2026-09-17
problem_type: api_design
category: architecture-patterns
severity: medium
tags: [ke-ownership, capability-flag, naming, carrier-anchored, viewpoint, owner-private, dual-SSOT, disclosure, scope]
---

# KE capability flag naming — anchor the flag to the carrier, not an invented layer

## Context

Spoke capability flags existed in two shapes before this axis: layer-anchored `l<N>-<concern>` (`l2-computable`, `l5-fork`, `l5-mind`) and named cross-layer families (`narrative-modules`, `spoke-connect`). When the KE ownership and extraction axes needed flags, the layer-prefixed form was the reflexive candidate (`l1-ownership`).

## Guidance

Name a capability flag after the concern's **actual carrier**:

- Use `l<N>-<concern>` **only** when the concern is genuinely anchored to one existing protocol layer (fork/mind → L5 timeline; computable → L2 body).
- When the concern spans layers (KE data fields + ops request/viewpoint semantics + library helpers), use a **named family** that names the carrier: `ke-<concern>`. The KE governance/extraction ADRs ruled `ke-ownership` and `ke-extraction` — independent flags, never an umbrella, no prerequisite ordering.
- Related rulings frozen with the flags: viewpoint selection lives on the **shared `Scope`** (one optional field consumed by check/assemble via existing `$ref` — no request-only sibling); `owner` references a holder KE `entry_id` (no reserved `world` token, no subject registry, no closed union); `owner-private` is documented vocabulary, **not** an enum.

## Why This Matters

Layer prefixes imply a normative layer definition that does not describe the concern; inventing one (`l1-ownership`) creates a false taxonomy that every later flag must fight. Carrier-anchored names keep the flag table honest and the capability negotiation legible.

## When to Apply

Any new capability flag. Check the concern against the existing layer definitions first; only anchor to a layer when the layer genuinely owns the whole concern.

## Examples

- `.mstar/specs/ke-ownership-disclosure-adr.md` §1.1 (naming rule + P2 destination), `.mstar/specs/ke-extraction-adr.md` §1.1.
- Related: `mind-axis-ownership-boundary.md` (dual-SSOT prevention for the mind axis), `capability-flagged-optional-bag.md` (shipping pattern).
