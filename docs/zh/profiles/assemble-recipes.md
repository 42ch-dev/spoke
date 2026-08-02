---
title: assemble 模块配方
---

# assemble 模块配方（Assemble module recipes）

本手册文档化可选 `AssemblePacket.modules` 字段袋下的两个**载荷级**配套方言：每条已组装条目偏好注入的位置，以及它为何被激活。

本页为[英文原页](/profiles/assemble-recipes)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## `modules.placement` —— 位置

逐条目注入提示：`entry_id` + `position_hint`（`before_defs`、`after_defs`、`depth`、`outlet`），带可选 `depth` / `outlet` 字段。词汇镜像 lore-activation，作者一次学会一种方言。数组顺序即互换顺序；主机读取提示后应用自己的布局。条目可省略 placement 行 —— 主机随后使用本地默认值。

## `modules.activation_trace` —— 原因

逐条目激活溯源：`entry_id` + `reason`（例如 `constant`、`key`），带可选匹配键细节 —— 记录哪条触发路径把条目放入载荷的调试 / 可观测记录。

## 集成方阅读

- 以 `entry_id` 将 `placement[]` / `activation_trace[]` 连接到 `entries[]`。
- 条目级 `modules.activation` 是偏好的持久创作处；载荷级 `placement[]` 是本包的组装快照。
- 基线 `AssemblePacket` 保持仅线上的精简条目；`modules` 经 `narrative-modules` 可选加入。

## 规范参考

- [assemble-module-recipes.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/assemble-module-recipes.md) —— 字段表、位置提示、激活轨迹、示例载荷
- [domain-profile-lore-activation.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-lore-activation.md) —— 共享位置提示词汇
- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) —— modules 归属权威
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) —— `assemble` 仅线上边界
