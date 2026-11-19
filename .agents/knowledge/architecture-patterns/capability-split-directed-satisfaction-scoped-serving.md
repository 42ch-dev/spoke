---
module: connect capability negotiation and serving
date: 2026-11-19
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

# Split a capability with directed satisfaction and scoped serving

## Context

A composite host profile often combines two different concerns: the local ports needed by orchestration and the remote operations a host may serve. Moving a useful subset behind a narrower capability can break existing hosts if the serving interface still requires the whole composite. Replacing the composite requirement with empty stubs is worse: it makes an unsupported host look operational.

## Guidance

1. **Separate local composition from remote authority.** Keep the existing local port composition intact for orchestration. Introduce a standalone serving port for the extracted operations instead of making the remote serving interface inherit the composite.
2. **Make compatibility a directed satisfaction rule.** The existing broad capability satisfies the new narrow requirement; the narrow capability never satisfies the broad one. Centralize that predicate and use it for both dispatch and peer selection. Do not apply the implication to independent authorization mechanisms such as exact capability-token grants.
3. **Represent serving as a set of optional faces.** A face contains only the provider and policy needed by that family. Constructors attach a face explicitly, and a probe reports genuine absence. An absent face follows the existing unsupported-operation refusal path; it never requires no-op implementations of unrelated ports.
4. **Resolve peer authority inside the serving seam.** Derive exactly one effective scope from the authenticated peer through admission before calling the provider. A request-carried scope is a declaration to check, not authority: require equality for identity-defining fields and permit only narrowing filters. Refuse unbound, ambiguous, or mismatched admission before provider access.
5. **Keep authorization refusal non-disclosing.** Use one generic refusal for admission and scope mismatch, with no resource-existence details and no state change. Preserve conditional capability checks and concurrency preconditions; extracting a capability must not weaken either.
6. **Verify the compatibility shape, not just the new path.** Cover the broad host serving the narrow operations, the narrow host serving them without composite stubs, and the narrow host refusing composite-only operations. Exercise two peers with the same resource identifier in different scopes, and assert both refusal bytes and zero provider calls across language implementations.

```text
required capability = narrow
satisfied = negotiated contains narrow OR negotiated contains broad

knowledge face = scoped provider + authenticated-peer admission
missing face = unsupported; denied admission = generic not-authorized
```

## Why This Matters

Directed satisfaction preserves existing negotiated sessions while allowing minimal hosts to advertise exactly what they own. Optional faces make absence explicit at the type boundary, and peer-derived admission prevents caller input from widening authority. Sharing the satisfaction predicate prevents dispatch and routing from selecting different answers for the same manifest.

## When to Apply

- A subset of a composite profile is useful to a host that cannot implement the rest.
- Existing broad-capability hosts must continue to satisfy the extracted requirement.
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
The broad capability satisfies the narrow requirement in one direction only.
```

Related: [`extract-op-injected-extraction.md`](extract-op-injected-extraction.md) for explicit optional service probes and distinguishing capability absence from application refusal; [`ke-capability-flag-naming.md`](ke-capability-flag-naming.md) for naming a capability after its carrier.
