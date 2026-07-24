# Knowledge

Compound outputs land here at iteration-close (`mstar-compound`). Durable protocol specs live in [`specs/`](../specs/).

## Index

| Doc | Category | Source | Summary |
|-----|----------|--------|---------|
| [`architecture-patterns/spoke-codegen-pipeline.md`](architecture-patterns/spoke-codegen-pipeline.md) | architecture-patterns | compound 2026-07-23 (bootstrap) | jstt+typify orchestrator, verify-codegen, rust-gen fail-fast, assemble-response `oneOf` |
| [`architecture-patterns/consumer-readme-twin.md`](architecture-patterns/consumer-readme-twin.md) | architecture-patterns | compound 2026-07-23 (dev-docs) | Twin EN/CN human READMEs; no delivery banners or anti-pattern rhetoric |
| [`architecture-patterns/timeline-projection-tiers.md`](architecture-patterns/timeline-projection-tiers.md) | architecture-patterns | compound 2026-07-23 (rule-event) | L5 TimelineScale brief/narrative/moment vocabulary |
| [`architecture-patterns/ops-response-error-oneof.md`](architecture-patterns/ops-response-error-oneof.md) | architecture-patterns | compound 2026-07-23 (ops-harden) | Ops response oneOf success\|error envelope; codegen barrel note |
| [`architecture-patterns/spoke-operations-pure-actions.md`](architecture-patterns/spoke-operations-pure-actions.md) | architecture-patterns | compound 2026-07-23 (operations-deepen) | Pure OCC/status/Scope/upsert/relate/error-map helpers over wire types |
| [`architecture-patterns/fixture-ajv-harness-outside-dist.md`](architecture-patterns/fixture-ajv-harness-outside-dist.md) | architecture-patterns | compound 2026-07-23 (fixtures-conformance) | AJV harness at `fixtures/toy-world/tests/` (`@42ch/spoke-fixture-toy-world`); operations package stays pure |
| [`architecture-patterns/knowledge-entry-timeline-event-vocabulary.md`](architecture-patterns/knowledge-entry-timeline-event-vocabulary.md) | architecture-patterns | compound 2026-07-23 (terminology) | KnowledgeEntry + TimelineEvent wire names; dual-concern vs `entry_type: "event"`; SPOKE = Knowledge Engine |
| [`architecture-patterns/l2-computable-session-model.md`](architecture-patterns/l2-computable-session-model.md) | architecture-patterns | compound 2026-07-24 (Computable) | Optional `l2-computable`: `body.state`/`computable`, Moment logs, Session lifecycle, `project`/`compute` ops |
| [`architecture-patterns/l5-fork-timeline-event-wire.md`](architecture-patterns/l5-fork-timeline-event-wire.md) | architecture-patterns | compound 2026-07-24 (Fork) | Optional `l5-fork`: `ForkId`, TimelineEvent `fork_id`/`parent_fork_id`, Scope.fork_id strict match |
