---
title: Assemble module recipes
---

# Assemble module recipes

This handbook documents two **packet-level** companion dialects under the optional `AssemblePacket.modules` bag: where each assembled entry prefers to inject, and why it was activated.

## `modules.placement` — where

Per-entry injection hints: `entry_id` + `position_hint` (`before_defs`, `after_defs`, `depth`, `outlet`), with optional `depth` / `outlet` fields. The vocabulary mirrors lore-activation so authors learn one dialect. Array order is the interchange order; hosts apply their own layout after reading the hints. An entry may omit a placement row — hosts then use a local default.

## `modules.activation_trace` — why

Per-entry activation provenance: `entry_id` + `reason` (for example `constant`, `key`) with optional matched-key detail — a debug / observability record of which fire path put the entry in the packet.

## Integrator reading

- Join `placement[]` / `activation_trace[]` to `entries[]` by `entry_id`.
- Entry-level `modules.activation` is the durable authoring home for preferences; packet-level `placement[]` is the assembled snapshot for this packet.
- Baseline `AssemblePacket` stays wire-only slim entries; `modules` is opt-in via `narrative-modules`.

## Normative references

- [assemble-module-recipes.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/assemble-module-recipes.md) — field tables, position hints, activation trace, illustrative packet
- [domain-profile-lore-activation.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-lore-activation.md) — shared position-hint vocabulary
- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) — modules placement authority
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) — `assemble` wire-only boundary
