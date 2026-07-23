# Adapters

Reserved directories for product mapping packages:

| Directory | Product |
|-----------|---------|
| `nexus/` | [Nexus](https://github.com/42ch/nexus) |
| `creader/` | [Creader](https://github.com/42ch/creader-editor) |

Each adapter maps product-native models ↔ SPOKE wire types (`@42ch/spoke-schema` / `spoke-schema`) and preserves `extensions.<namespace>` on round-trip. Schemas in [`schemas/`](../schemas/) remain the protocol SSOT.

See root [`README.md`](../README.md), [`AGENTS.md`](../AGENTS.md), and [`.mstar/specs/spoke-protocol.md`](../.mstar/specs/spoke-protocol.md).
