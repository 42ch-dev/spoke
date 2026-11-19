---
module: connect capability negotiation and serving
date: 2026-11-19
last_updated: 2026-11-19
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
tags:
  - capability-split
  - negotiation
  - serving-faces
  - scope-admission
  - silent-refusal
applies_when:
  - extracting remotely useful operations from a composite host capability
  - serving peer-scoped data through an independently attachable provider face
---

# Split a capability with explicit negotiation and scoped serving

## Context

A composite host profile often combines two different concerns: the local ports needed by orchestration and the remote operations a host may serve. Moving a useful subset behind a narrower capability can break existing hosts if the serving interface still requires the whole composite. Replacing the composite requirement with empty stubs is worse: it makes an unsupported host look operational.

## Guidance

1. **Separate local composition from remote authority.** Keep the existing local port composition intact for orchestration. Introduce a standalone serving port for the extracted operations instead of making the remote serving interface inherit the composite.
2. **Make negotiation exact and explicit.** The extracted operations require their own capability in `negotiated_capabilities`; the existing broad capability authorizes only its original operations and never satisfies the narrow one. A host that previously relied on the broad declaration must add the narrow capability and attach the scoped face. Use the same exact-membership predicate for dispatch, peer selection and independent capability-token grants.
3. **Represent serving as a set of optional faces.** A face contains only the provider and policy needed by that family. Constructors attach a face explicitly, and a probe reports genuine absence. An absent face follows the existing unsupported-operation refusal path; it never requires no-op implementations of unrelated ports.
4. **Resolve peer authority inside the serving seam.** Derive exactly one effective scope from the authenticated peer through admission before calling the provider. A request-carried scope is a declaration to check, not authority: require equality for identity-defining fields. Allow requested `entry_ids`, `timeline_event_ids` and `entry_types` only when they are subsets of the admitted sets, and pass those subsets to the provider. Refuse unbound or ambiguous admission, identity mismatch, or any value outside its admitted set before provider access.
5. **Keep authorization refusal non-disclosing.** Use one generic refusal for admission and scope mismatch, with no resource-existence details and no state change. Preserve conditional capability checks and concurrency preconditions; extracting a capability must not weaken either.
6. **Verify the explicit-authority shape, not just the new path.** Cover the broad host adding the narrow declaration and scoped face, the narrow host serving without composite stubs, and the broad declaration alone refusing the narrow operations. Exercise two peers with the same resource identifier in different scopes, subset filters accepted and outside values refused before provider access, and assert both refusal bytes and zero provider calls across language implementations.

```text
required capability = narrow
satisfied = negotiated contains narrow

knowledge face = scoped provider + authenticated-peer admission
subset filters = entry_ids / timeline_event_ids / entry_types within admitted sets
missing face = unsupported; denied admission or outside filter = generic not-authorized
```

## Why This Matters

Explicit negotiation keeps each capability's authority precise while allowing minimal hosts to advertise exactly what they own. Optional faces make absence explicit at the type boundary, and peer-derived admission prevents caller input from widening authority. Sharing exact-membership checks prevents dispatch, routing and token authorization from selecting different answers for the same manifest.

## When to Apply

- A subset of a composite profile is useful to a host that cannot implement the rest.
- Each operation family must be authorized only by its own explicitly negotiated capability.
- Remote operations need a peer-specific scope that must not become caller-controlled wire authority.
- A serving interface needs independently attachable families without placeholder providers.

## Examples

### Before

```text
Remote serving requires the complete local composite.
A storage-only host either advertises too much or installs empty handlers.
```

### After

```text
Local orchestration keeps its composite.
Remote serving exposes an optional scoped face.
The narrow capability is explicitly negotiated; the broad capability never implies it.
```

Related: [`extract-op-injected-extraction.md`](extract-op-injected-extraction.md) for explicit optional service probes and distinguishing capability absence from application refusal; [`ke-capability-flag-naming.md`](ke-capability-flag-naming.md) for naming a capability after its carrier.
