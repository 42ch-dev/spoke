# Adapters

Product mapping packages will live under `adapters/`. Each adapter translates a product’s native models to and from SPOKE wire types (`@42ch/spoke-schemas` / `spoke-schemas`), keeps `extensions.<namespace>` intact on round-trip, and calls `@42ch/spoke-operations` for shared lifecycle rules.

Schemas in [`schemas/`](../schemas/) remain the protocol SSOT. See [`AGENTS.md`](../AGENTS.md) and [`.mstar/specs/spoke-protocol.md`](../.mstar/specs/spoke-protocol.md).
