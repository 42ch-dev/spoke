---
title: 操作库
---

# 操作库（Operations library）

操作库是生成线上类型之上的手写行为层：纯函数生命周期辅助，加上按能力切片的 adapter ports 与注入式编排。它双份发布 —— `@42ch/spoke-operations`（TypeScript）与 `spoke-operations`（Rust）—— 在锁步 SemVer 下行为对齐。

本页为[英文原页](/guide/operations-library)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 纯函数辅助

- 扩展与模块映射合并 + 往返保留（`mergeExtensionMaps`、`preserveModuleMaps` 等）。
- Finding 状态迁移校验与应用。
- 晋升接受门控（provisional 要求、终态拒绝、revision 递增）—— 纯且预持久化。
- KnowledgeEntry 状态迁移，以及调用方提供集合上的活跃唯一性检查。
- KnowledgeEntry 与 TimelineEvent 的 Scope 匹配辅助；时间线排序辅助（`orderTimelineEventsByPrecedes` 等）。
- AssemblePacket 构建器，带保序截断。
- 所有拒绝路径上统一的 `SpokeResult` / `SpokeRejectCode` —— 预期拒绝返回 `SpokeReject` 而非抛出。

## Adapter ports 与编排

实现所声明能力要求的 port 族（`KnowledgeEntryPort`、`RelationPort`、`ScopeQueryPort`、`FindingPort`、`RuleQueryPort`、`HostManifestPort`，加可选 `ComputablePort` / `ForkTimelineQueryPort`），再调用对应编排器：

```ts
// 仅示意 —— 完整签名见软件包 README。
import { orchestrateUpsert, type BaselineAdapter } from "@42ch/spoke-operations";

declare const adapter: BaselineAdapter; // 产品 adapter 实现各 ports
// orchestrateUpsert(adapter, request) → 加载 scope、执行门控、经 ports 持久化
```

组合别名（`BaselineAdapter`、`ComputableAdapter`、`ForkAdapter`、`FullAdapter`）按能力等级命名 port 交集。主机协作经 `HostManifestPort` 运行（`getHostCapabilityManifest`、`listPeerHostCapabilityManifests`）。

## 纯函数边界

该库保持纯净：存储 I/O、LLM 调用、排序、检索与传输绑定由产品通过注入 ports 与 adapter 提供。参考 adapter 与一致性演示位于 `fixtures/toy-world/`。

## 规范参考

- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) —— 辅助契约、adapter 接口、注入式编排
- [@42ch/spoke-operations README](https://github.com/42ch-dev/spoke/blob/main/packages/spoke-operations/README.md) —— TypeScript 用法
- [spoke-operations README (Rust)](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-operations/README.md) —— Rust 用法
- [fixtures/toy-world/](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) —— 参考 adapter 示例与一致性图
