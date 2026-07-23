# Concepts — SPOKE Domain Vocabulary

Core terms for the SPOKE protocol repository. Each entry defines what the term means *in SPOKE wire form*, not inside any single product runtime.

---

## Protocol layer

### Keyblock
The atomic narrative knowledge unit on the SPOKE wire. A Keyblock has stable identity (`keyblock_id`), open-string `block_type` and `status`, a structured `body`, optional provenance (`source_anchor`), and required `extensions`. Products map their local entities to Keyblocks via adapters (next iteration).

### Relation
A directed edge between two Keyblocks (or Keyblock ↔ source) identified by `relation_id` and open-string `relation_type`.

### SourceAnchor
A pointer to a source artifact span (manuscript, scene, external locator). Ties Keyblocks and Findings to provenance without embedding product file paths in protocol fields.

### Finding
Checker **output** — consistency, style, structure, or other analysis results. Not a Keyblock body and not a declarative rule definition.

### AssemblePacket
Wire-only context-assembly payload: a list of slim entries (`keyblock_id`, `block_type`, `canonical_name`, optional `snippet`). Ranking, retrieval, and token budgeting are product-local; see [`spoke-ops.md` §assemble](.mstar/specs/spoke-ops.md#assemble-wire-only-boundary-normative).

### Extensions (`extensions.<namespace>`)
The sole product-specific bag on every data object. Namespace keys are product ids (`nexus`, `creader`, …). Adapters MUST round-trip unknown namespaces and keys verbatim.

---

## Boundaries (what SPOKE is not)

| Term | In SPOKE | Product-local (not redefined here) |
|------|----------|-------------------------------------|
| **Keyblock** | Portable wire envelope | — |
| **World KB** | Mapped *to* Keyblocks via adapters | Nexus world's structured knowledge store |
| **Author Memory** | May appear under `extensions.<namespace>` | Creator profile, SOUL, session memory pipelines |
| **Rule** | **Deferred v0.1** — no wire schema | Creader `KnowledgeEntryType: "rule"`, Nexus `rule_suggestion` on findings |

**Invariant:** SPOKE standardizes interchange shapes. It does not own world history, fork semantics, daemon routes, or checker engines.

---

## Spelling

| Context | Spelling |
|---------|----------|
| SPOKE protocol / schemas / packages | **Keyblock** |
| Nexus product code / UI | **KeyBlock** (product spelling) |
| Creader knowledge entries | **KnowledgeEntry** (mapped in adapters later) |

---

## See also

| Doc | Topic |
|-----|-------|
| [`STRATEGY.md`](STRATEGY.md) | Protocol-not-runtime positioning |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella spec |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | Data objects and open vocabulary |
