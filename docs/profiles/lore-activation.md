---
title: Domain Profile — lore activation
---

# Domain Profile — lore activation

This handbook defines **lore activation**: the fire conditions under which a KnowledgeEntry prefers to surface into assembled context. It lives as the `modules.activation` inner dialect on the optional `modules` bag (capability-flagged `narrative-modules`) — matching, scanning, and ranking stay product-local.

## What `modules.activation` carries

- **`keys`** — primary activation triggers (aliases, names, phrases).
- **`secondary_keys`** + **`logic`** — selective combinations (`and_any`, `and_all`, `not_any`, `not_all`).
- **`constant`** — marks an always-on seed candidate (valid with empty `keys`).
- **`order`** / **`priority`** — insertion and tie-break hints.
- **`position_hint`** / **`outlet`** — preferred placement (`before_defs`, `after_defs`, `depth`, `outlet`).
- **`match`** — literal, regex, or whole-word key matching (flavor defined by the product scanner).

## Key invariants

- **Standalone snippet** — `body.summary` and `AssemblePacket` entry `snippet` read as complete lore facts without the trigger keys.
- **Seed vs Pool** — `constant: true` entries feed the always-on seed set; keyed entries feed the activation pool (full pattern in the Knowledge Pack handbook).
- **Relation-first recursion** — lore adjacency expands through `Relation` edges; string key-mention recursion is a migration-only path.

## Engine boundary

Keyword matching, scan windows, token budgets, and ranking are product-local. The protocol carries the fire conditions for round-trip and pack import; `spoke-operations` provides no matchers.

## Normative references

- [domain-profile-lore-activation.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-lore-activation.md) — field table, logic values, position hints, match modes, integrator checklist
- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) — modules placement authority
- [domain-profile-narrative-knowledge-pack.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-narrative-knowledge-pack.md) — companion handbook (Seed vs Pool)
- [assemble-module-recipes.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/assemble-module-recipes.md) — packet-level placement / activation trace
