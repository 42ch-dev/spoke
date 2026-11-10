# SPOKE Specs

Current-state specification of the SPOKE protocol. Each page states what the system is now; decision history lives in [`.agents/adr/`](../adr/), and reusable engineering experience lives in [`.agents/knowledge/`](../knowledge/).

Wire authority stays in root [`schemas/`](../../schemas/); these pages describe the contracts, invariants and paths around it.

| Spec | Summary |
|------|---------|
| [`protocol/spoke-protocol.md`](protocol/spoke-protocol.md) | Umbrella protocol: three columns, L0–L8 model, schema URI rules, language packages, repository layout |
| [`protocol/spoke-protocol-layers.md`](protocol/spoke-protocol-layers.md) | Layer model and capability levels (baseline + optional flags) |
| [`protocol/spoke-data-model.md`](protocol/spoke-data-model.md) | Data wire shapes: KnowledgeEntry, Rule, TimelineEvent, TimelineScale, host manifest, MindState |
| [`protocol/spoke-ops.md`](protocol/spoke-ops.md) | Ops wire layer: request/response families, optional ops, error envelope |
| [`protocol/spoke-operations.md`](protocol/spoke-operations.md) | Operations library: pure helpers, adapter ports, injection orchestration |
| [`protocol/spoke-version-release.md`](protocol/spoke-version-release.md) | Lockstep SemVer, tag convention, CI-gated release policy |
| [`connect/spoke-connect.md`](connect/spoke-connect.md) | Connect envelopes, identity binding, session-core states, dispatch gate |
| [`connect/noise-xx-libp2p-contract.md`](connect/noise-xx-libp2p-contract.md) | Frozen Noise XX + libp2p framing wire contract |
| [`connect/connect-publish-strategy.md`](connect/connect-publish-strategy.md) | Published surface inventory, registry channels, auth, binding disposition |
| [`connect/connect-cpp-binding.md`](connect/connect-cpp-binding.md) | C/C++ carrier ABI, C++17 convenience layer, native artifact rules |
| [`connect/connect-csharp-binding.md`](connect/connect-csharp-binding.md) | C# binding surface, regenerate/build/run, fork drop condition |
| [`memory/spoke-memory.md`](memory/spoke-memory.md) | `spoke-memory` family: models, invariants, 33 ops, wire mapping |
| [`cognition/spoke-cognition-data-contract.md`](cognition/spoke-cognition-data-contract.md) | Cognition records, attribution, projections, Memory carrier, pure helpers |
| [`cognition/spoke-cognition-consumer-contract.md`](cognition/spoke-cognition-consumer-contract.md) | Served-surface conformance, negotiation, library ownership |
| [`domain-profiles/domain-profile-narrative-structure.md`](domain-profiles/domain-profile-narrative-structure.md) | Narrative-structure profile: Beat senses, `precedes`, `structural_role` |
| [`domain-profiles/domain-profile-lore-activation.md`](domain-profiles/domain-profile-lore-activation.md) | Lore activation under `modules.activation` |
| [`domain-profiles/domain-profile-mental-state.md`](domain-profiles/domain-profile-mental-state.md) | Mental-state vocabulary: `modules.mental`, `modules.observation`, MindState |
| [`domain-profiles/domain-profile-narrative-knowledge-pack.md`](domain-profiles/domain-profile-narrative-knowledge-pack.md) | Narrative Knowledge Pack interchange pattern |
| [`domain-profiles/assemble-module-recipes.md`](domain-profiles/assemble-module-recipes.md) | AssemblePacket `modules.placement` / `modules.activation_trace` recipes |
| [`extensions/spoke-extension-modules.md`](extensions/spoke-extension-modules.md) | Core / `modules.*` / `extensions.<product>` placement triad |
| [`adapters/spoke-remote-adapter.md`](adapters/spoke-remote-adapter.md) | RemoteAdapter surface, Transport seam, multi-peer router, FFI parity |

Index rows are prose navigation only; the harness catalog is the registry.
