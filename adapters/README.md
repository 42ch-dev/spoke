# Adapters (deferred)

v0.1 reserves product-specific mapping homes under `adapters/`. **No adapter packages ship in this iteration** — directories contain `.gitkeep` only. There is no `package.json`, `Cargo.toml`, or conversion code here, and nothing in the repo depends on adapters at runtime.

## Next iteration (planned)

| Directory | Product | Scope (draft) |
|-----------|---------|---------------|
| `nexus/` | [Nexus](https://github.com/42ch/nexus) | Map Nexus narrative/KB wire shapes ↔ SPOKE Keyblock data + ops envelopes; preserve `extensions.nexus` on round-trip. |
| `creader/` | [Creader](https://github.com/42ch/creader-editor) | Map Creader editor/knowledge-base entities ↔ SPOKE Keyblock data + ops envelopes; preserve `extensions.creader` on round-trip. |

Each adapter will be a **standalone mapping package** (TypeScript and/or Rust TBD) that:

1. Translates product-native models into SPOKE wire types from `@42ch/spoke-schema` / `spoke-schema`.
2. Translates SPOKE responses back without dropping unknown extension namespaces or keys.
3. Stays out of the protocol SSOT — schemas remain authoritative in `schemas/`.

Conformance fixtures and golden round-trips are also deferred until adapter packages exist.

## v0.1 contract

- **In scope:** visible placeholder paths (`adapters/nexus/`, `adapters/creader/`).
- **Out of scope:** implementation, CI wiring, publish, or dependency edges from codegen packages.

See root [`README.md`](../README.md), [`AGENTS.md`](../AGENTS.md), and [`.mstar/specs/spoke-protocol.md`](../.mstar/specs/spoke-protocol.md) for protocol identity and layering.
