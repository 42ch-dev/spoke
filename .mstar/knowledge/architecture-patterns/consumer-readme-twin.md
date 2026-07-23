# Consumer README twin pattern (EN/CN)

**Category:** architecture-patterns  
**Source:** iteration:v0-iter002 (dev-docs)  
**Status:** durable

## Problem

Protocol repos often mix maintainer process docs with integrator onboarding. Dual-language pages drift when section outlines differ or terminology (e.g. wire translations) is inconsistent.

## Decision

1. **Audience:** Root `README.md` / `README_CN.md` target **protocol consumers**, not harness maintainers.
2. **Twin structure:** Same section outline and tables; cross-link at the top.
3. **Version labels:** Separate bootstrap wire (v0.1) from later slices (e.g. `v0-iter002` operations) — do not bundle newer packages under older ship banners.
4. **Spelling:** Protocol token **Keyblock** stays English in CN docs.

## Related

- Specs: `.mstar/specs/spoke-protocol.md`, `spoke-operations.md`
- Packages: `@42ch/spoke-schema`, `@42ch/spoke-operations`
