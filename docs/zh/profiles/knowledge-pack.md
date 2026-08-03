---
title: 知识包
---

# Domain Profile —— 知识包（Knowledge pack）

**Narrative Knowledge Pack（叙事知识包）** 是可移植的世界观捆绑：一组有序的 KnowledgeEntry、Relation 与可选 SourceAnchor，在叙事主机之间传递。包使用既有线上原子，外加携带目录元数据（`title` / `version` / `creator`）的**产品传输信封**。包 ≠ AssemblePacket（持久库互换 vs 临时组装输出）。

本页为[英文原页](/profiles/knowledge-pack)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 包方言

- **原子** —— 既有线上对象：KnowledgeEntry（世界观节点）、Relation（图边）、SourceAnchor（可选溯源）。
- **包目录元数据** —— 产品信封字段：`title`、`version`、`creator`、可选 `description`（不在 KE 或 AssemblePacket 的 `modules.*` 上）。
- **逐条目触发条件** —— 当包携带激活时，`modules.activation` 随每条 KnowledgeEntry 同行（lore-activation 手册）。
- **往返** —— 导入方原样保留未知 `extensions` namespace、未知模块键与开放字符串词汇。

## 组合 / 堆叠模型

叙事主机在产品侧堆叠多个包（世界包、角色包、会话包）：按导出顺序导入原子、按 id 合并 Relation 图、并集种子与池集合，然后在合并后运行激活 / 范围 / 预算。堆叠策略（优先级、覆盖、软删除）在产品侧。

## 种子与池

完整组装候选模式在本手册：常开**种子**条目（`constant: true`）加带键**池**条目、调用方提供候选顺序、经纯函数辅助的 `maxEntries` 截断。

## 规范参考

- [domain-profile-narrative-knowledge-pack.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-narrative-knowledge-pack.md) —— 包方言、组合指引、往返规则、种子与池
- [domain-profile-lore-activation.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-lore-activation.md) —— `modules.activation` 字段表
- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) —— modules 归属权威
- [fixtures/toy-world/](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) —— 一致性原子与配套包样例
