# Consumer README twin pattern (EN/CN)

**Category:** architecture-patterns  
**Source:** compound 2026-07-23 (dev-docs)  
**Status:** durable

## Problem

Protocol repos often mix maintainer process docs with integrator onboarding. Dual-language pages drift when section outlines differ or terminology (e.g. wire translations) is inconsistent.

## Decision

1. **Audience:** Root `README.md` / `README_CN.md` are **for humans** (protocol consumers), not agent steering docs.
2. **Twin structure:** Same section outline and tables; cross-link at the top.
3. **No iteration delivery in README:** Affirmative product state only — no iteration IDs or ship banners (root `AGENTS.md` § README audience).
4. **No anti-pattern rhetoric in README:** Avoid “not a…”, “does not include…”, “out of scope…” exclusion prose; put boundaries in root `AGENTS.md` (or normative specs).
5. **Spelling:** Protocol token **KnowledgeEntry** stays English in CN docs (do not translate wire type names).

## Related

- Specs: `.mstar/specs/spoke-protocol.md`, `spoke-operations.md`
- Packages: `@42ch/spoke-schemas`, `@42ch/spoke-operations`
