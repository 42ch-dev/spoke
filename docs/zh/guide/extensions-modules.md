---
title: 扩展与模块
---

# 扩展与模块（Extensions & modules）

集成方添加到 SPOKE 对象的每个字段都属于三个字段袋之一：**核心字段**（协议所有，封闭）、**`modules.*`**（可选跨产品功能方言）、**`extensions.<namespace>`**（单个产品的私有字段袋）。归属权威是规范性 ADR；本页是快速参考。

本页为[英文原页](/guide/extensions-modules)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 三分法

- **核心字段** —— 协议身份与封闭 body 信封（`entry_id`、`body.summary`、`schema_version`）；每个基线主机读写它们。
- **`modules.*`** —— KnowledgeEntry 与 AssemblePacket 上可选、能力标志（`narrative-modules`）的字段袋，承载跨产品功能方言：`modules.activation`（KE 上的 lore activation）、`modules.placement` / `modules.activation_trace`（AssemblePacket 上的 assemble 配方）。内部形状由手册定义；未知模块键原样往返。知识包的**目录**元数据位于产品传输信封 —— 而非 KE 上的 `modules.pack`。
- **`extensions.<namespace>`** —— 每个持久数据对象上必填的 `ExtensionMap`；namespace 键是产品不透明 id（`^[a-z][a-z0-9_-]*$`），值是任意 JSON。adapter 对未知 namespace 与键原样往返。

## 归属规则

**跨产品功能方言**用 `modules.*`；**产品数据**用 `extensions.<product>`。在 `extensions.*` 下发布共享功能键会与 namespace 的产品 id 解读及 `HostCapabilityManifest.namespaces[]` 排他性冲突 —— 因此 activation、placement 与 activation_trace 位于 `modules.*`。包目录元数据留在产品传输信封。

## 主机清单

在 `HostCapabilityManifest` 上，`extensions` 携带部署元数据；roles、capabilities 与 namespace 归属是核心清单字段。

## 规范参考

- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) —— 三分法 ADR：归属规则、往返、权威范围
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— Extensions / Modules 词汇
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) —— 数据对象上的扩展契约与 ModuleMap
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) —— `narrative-modules` 能力标志
- [common.schema.json](https://github.com/42ch-dev/spoke/blob/main/schemas/common/common.schema.json) —— `ExtensionMap` / `ModuleMap` 定义
