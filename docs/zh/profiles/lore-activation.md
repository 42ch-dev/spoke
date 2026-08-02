---
title: 世界观激活（lore activation）
---

# Domain Profile —— 世界观激活（Lore activation）

本手册定义 **lore activation（世界观激活）**：KnowledgeEntry 在哪些触发条件下更倾向于浮现进组装的上下文。它作为可选 `modules` 字段袋（能力标志 `narrative-modules`）上的 `modules.activation` 内部方言存在 —— 匹配、扫描与排序留在产品侧。

本页为[英文原页](/profiles/lore-activation)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## `modules.activation` 携带的内容

- **`keys`** —— 主要激活触发器（别名、名称、短语）。
- **`secondary_keys`** + **`logic`** —— 选择性组合（`and_any`、`and_all`、`not_any`、`not_all`）。
- **`constant`** —— 标记常开种子候选（可与空 `keys` 共存）。
- **`order`** / **`priority`** —— 插入与并列打破提示。
- **`position_hint`** / **`outlet`** —— 首选摆放（`before_defs`、`after_defs`、`depth`、`outlet`）。
- **`match`** —— 字面、正则或整词键匹配（匹配风格由产品扫描器定义）。

## 关键不变量

- **独立片段** —— `body.summary` 与 `AssemblePacket` 条目的 `snippet` 在无触发键的情况下读作完整世界观事实。
- **种子与池** —— `constant: true` 条目进入常开种子集；带键条目进入激活池（完整模式见知识包手册）。
- **关系优先递归** —— 世界观邻近经 `Relation` 边扩展；字符串键提及递归是迁移路径。

## 引擎边界

关键词匹配、扫描窗口、令牌预算与排序在产品侧；协议携带触发条件用于往返与包导入，匹配由产品实现。

## 规范参考

- [domain-profile-lore-activation.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-lore-activation.md) —— 字段表、logic 值、位置提示、匹配模式、集成方清单
- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) —— modules 归属权威
- [domain-profile-narrative-knowledge-pack.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-narrative-knowledge-pack.md) —— 配套手册（种子与池）
- [assemble-module-recipes.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/assemble-module-recipes.md) —— 载荷级摆放 / 激活轨迹
