# Grill lock — 2026-07-23

Interactive direction lock for SPOKE v0.1 (iteration-start).

## Locked answers

1. **Branch policy** — `iteration_base_branch=main`, `spec_integration_branch=iteration/v0.1`, `target_branch=main`
2. **Plan split** — single plan; adapters = empty directories (not packages)
3. **Package naming** — `spoke-schema` (avoid "contracts" / smart-contract connotation)
4. **Extensions** — explicit `extensions` object keyed by product namespace; core fields `additionalProperties: false`
5. **Codegen** — TypeScript `json-schema-to-typescript` + Rust `typify`

## Research input

Cross-product survey canvas (spoke-group workspace): Nexus **KeyBlock** (product spelling) + Creader KnowledgeEntry/Guardian/ContextPacket → SPOKE two-layer protocol (data + ops). SPOKE wire spelling: **Keyblock**.

See also: [`delivery-compass.md`](../delivery-compass.md).
