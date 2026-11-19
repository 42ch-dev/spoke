# ADR: Split remote knowledge serving from baseline ports
Status: implemented — the scoped knowledge face and explicit capability negotiation govern connect serving
Date: 2026-11-19
Scope: SPOKE operations port composition, connect dispatch and responder serving, native bindings

## Problem

Remote knowledge reads and writes need a peer-specific authorization scope, while baseline orchestration also needs unscoped knowledge persistence and query ports. Combining those concerns makes a network peer's declared Scope indistinguishable from the host's admitted Scope and prevents a host from exposing knowledge access as its only remote service.

## Decision

`KnowledgeQueryPort` owns baseline knowledge listing and `ScopeQueryPort` owns baseline timeline listing; both compose into `BaselinePorts`. `ScopedKnowledgePort` is a standalone connect-serving port whose get, put, and list methods receive an admitted `Scope`.

The three remote knowledge ops require `spoke-knowledge` explicitly in `negotiated_capabilities`. `spoke-baseline` authorizes only baseline ops and never satisfies the knowledge requirement. A host that previously relied on its baseline declaration adds `spoke-knowledge` and attaches the scoped knowledge face. Capability-token grants also name `spoke-knowledge` explicitly for those ops; grant matching remains exact membership.

A responder attaches a `RemoteKnowledgeFace` containing the scoped store and an admission callback for the authenticated peer. Knowledge get and put use the admitted Scope directly. Knowledge listing requires exact `scope_id` and `viewpoint` equality. Requested `entry_ids`, `timeline_event_ids` and `entry_types` may be subsets of the admitted sets and are passed to the store; any value outside its admitted set refuses `NOT_AUTHORIZED` before the store. Admission refusals and scope mismatches return `NOT_AUTHORIZED` without entry-existence details. Native bindings expose the same face through appended ABI revision 2 callbacks.

## Alternatives considered

- **Keep knowledge methods inside the baseline serving face.** This preserves one provider object but leaves no standalone knowledge-serving boundary and mixes local baseline composition with remote admission. The split keeps baseline orchestration intact while making remote scope authority explicit.
- **Give knowledge-only hosts `spoke-baseline`.** This reuses the existing flag but advertises unrelated baseline operations and prevents precise routing. A distinct, explicitly negotiated flag supports both baseline hosts and knowledge-only hosts without implying either capability from the other.
- **Authorize knowledge with a request-carried Scope.** This makes caller input authoritative at the trust boundary. Authenticated-peer admission instead owns `scope_id` and `viewpoint`, while `entry_ids`, `timeline_event_ids` and `entry_types` are accepted only as subsets of the admitted sets.
- **Create a new C ABI table.** A separate table would duplicate responder construction and callback ownership. Appending the admission and scoped methods preserves one ports table and makes the layout revision explicit.

## Consequences

Hosts serving remote knowledge implement `ScopedKnowledgePort` and admission alongside any baseline ports they expose, and declare `spoke-knowledge` explicitly even when they also declare `spoke-baseline`. Routers, responders and token authorization all use exact capability membership. Requested filter sets narrow admitted sets only; an outside value refuses before provider access. Native binding implementors provide the appended callbacks at ABI revision 2. Exact admitted-viewpoint matching deliberately refuses requests that omit or alter that reader context.
