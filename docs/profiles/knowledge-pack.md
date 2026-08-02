---
title: Domain Profile — knowledge pack
---

# Domain Profile — knowledge pack

A **Narrative Knowledge Pack** is a portable lore bundle: an ordered set of KnowledgeEntries, Relations, and optional SourceAnchors that travel between narrative hosts. Packs use existing wire atoms plus the `modules.pack` metadata dialect on the optional `modules` bag — a dedicated pack container envelope stays product-local.

## Pack dialect

- **Atoms** — existing wire objects: KnowledgeEntry (lore nodes), Relation (graph edges), SourceAnchor (optional provenance).
- **Pack metadata** — `modules.pack`: `title`, `version`, `creator`, optional `description`.
- **Per-entry fire conditions** — `modules.activation` travels with each KnowledgeEntry when the pack carries activation (lore-activation handbook).
- **Round-trip** — importers preserve unknown `extensions` namespaces, unknown module keys, and open-string vocabulary verbatim.

## Compose / stack model

Narrative hosts stack multiple packs product-side (world pack, character pack, session pack): import atoms preserving export order, merge Relation graphs by id, union seed and pool sets, then run activation / scope / budget after the merge. Stack policy (priority, override, soft-delete) is product-local.

## Seed vs Pool

The full assemble candidate pattern lives in this handbook: always-on **seed** entries (`constant: true`) plus keyed **pool** entries, caller-supplied candidate order, and `maxEntries` truncation through pure helpers.

## Normative references

- [domain-profile-narrative-knowledge-pack.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-narrative-knowledge-pack.md) — pack dialect, compose guidance, round-trip rules, Seed vs Pool
- [domain-profile-lore-activation.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-lore-activation.md) — `modules.activation` field table
- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) — modules placement authority
- [fixtures/toy-world/](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) — conformance atoms and companion pack samples
