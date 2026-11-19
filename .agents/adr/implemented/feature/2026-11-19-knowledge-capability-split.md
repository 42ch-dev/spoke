# ADR: Split remote knowledge serving from baseline ports
Status: implemented — the scoped knowledge face and directed capability rule govern connect serving
Date: 2026-11-19
Scope: SPOKE operations port composition, connect dispatch and responder serving, native bindings

## Problem

Remote knowledge reads and writes need a peer-specific authorization scope, while baseline orchestration also needs unscoped knowledge persistence and query ports. Combining those concerns makes a network peer's declared Scope indistinguishable from the host's admitted Scope and prevents a host from exposing knowledge access as its only remote service.

## Decision

`KnowledgeQueryPort` owns baseline knowledge listing and `ScopeQueryPort` owns baseline timeline listing; both compose into `BaselinePorts`. `ScopedKnowledgePort` is a standalone connect-serving port whose get, put, and list methods receive an admitted `Scope`.

The three remote knowledge ops require `spoke-knowledge`. `spoke-baseline` satisfies that requirement for negotiated dispatch and router selection, and the implication is directed. Capability-token grants name `spoke-knowledge` explicitly for those ops.

A responder attaches a `RemoteKnowledgeFace` containing the scoped store and an admission callback for the authenticated peer. Knowledge get and put use the admitted Scope directly. Knowledge listing requires exact `scope_id` and `viewpoint` equality, then applies the request's remaining refinements to that admitted Scope. Admission refusals and scope mismatches return `NOT_AUTHORIZED` without entry-existence details. Native bindings expose the same face through appended ABI revision 2 callbacks.

## Alternatives considered

- **Keep knowledge methods inside the baseline serving face.** This preserves one provider object but leaves no standalone knowledge-serving boundary and mixes local baseline composition with remote admission. The split keeps baseline orchestration intact while making remote scope authority explicit.
- **Give knowledge-only hosts `spoke-baseline`.** This reuses the existing flag but advertises unrelated baseline operations and prevents precise routing. A distinct flag with baseline-to-knowledge implication supports both baseline hosts and knowledge-only hosts.
- **Authorize knowledge with a request-carried Scope.** This makes caller input authoritative at the trust boundary. Authenticated-peer admission instead owns `scope_id` and `viewpoint`, while the request supplies only matching values and narrowing refinements.
- **Create a new C ABI table.** A separate table would duplicate responder construction and callback ownership. Appending the admission and scoped methods preserves one ports table and makes the layout revision explicit.

## Consequences

Hosts serving remote knowledge implement `ScopedKnowledgePort` and admission alongside any baseline ports they expose. Routers and responders share the directed capability-satisfaction rule, while token authorization remains exact membership. Native binding implementors provide the appended callbacks at ABI revision 2. Exact admitted-viewpoint matching deliberately refuses requests that omit or alter that reader context.
