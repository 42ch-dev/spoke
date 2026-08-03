---
layout: home
hero:
  name: SPOKE
  text: Standardized Programmable Ontology Knowledge Engine
  tagline: 集成文档 —— 线上契约、操作、Domain Profile（领域画像）与 connect（连接）绑定。
  actions:
    - { theme: brand, text: 快速开始, link: /zh/packages/quick-start }
    - { theme: alt, text: 阅读协议, link: /zh/guide/protocol }
features:
  - { title: 协议与数据模型, details: "KnowledgeEntry、Relation、Finding 与 ops 线上契约 —— 面向集成方的概览，并链接到规范说明。", link: /zh/guide/data-model }
  - { title: Domain Profile（领域画像）, details: "叙事结构、lore activation（世界观激活）、知识包与 assemble 配方。", link: /zh/profiles/narrative-structure }
  - { title: connect（连接）, details: "可选的 connect 信封族 —— TypeScript 路线与原生绑定路线。", link: /zh/connect/overview }
  - { title: 软件包, details: "npm 与 crates.io 的安装固定，采用锁步 SemVer。", link: /zh/packages/quick-start }
---

## 为什么选择 SPOKE

SPOKE 是一套面向叙事知识的 JSON Schema 线上契约协议：各独立产品通过共享的数据与 ops 形状交换一致性检查和上下文组装的 I/O，避免每个产品为同一概念各自发明本地格式。本仓库即单一事实来源（SSOT）—— 手写 schema、`.mstar/specs/` 中的规范说明、生成的 TypeScript 与 Rust 线上类型，以及纯函数操作库。

- **单一线上方言** —— 八个数据对象与五个基线 ops（另含可选 `project` / `compute`）
- **基于开放字符串的 Domain Profile** —— 本体词汇经开放字符串发布，并经可选 `modules.*` 字段袋承载跨产品方言
- **能力标志** —— 声明 `spoke-baseline`，或显式声明 `l2-computable`、`l5-fork`、`narrative-modules`、`spoke-connect`
- **语言对齐** —— npm 与 crates.io 共用一套锁步 SemVer
- **可选 connect** —— 已签名的跨进程交互信封，带按会话排序与可扩展鉴权

## 规范参考

仓库内的规范说明是单一事实来源；本站页面对其进行概览并链接到原文。

- [spoke-protocol.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol.md) —— 总览：三列模型、schema 清单、扩展契约
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— 协议词汇，以线上术语定义
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) —— 九层模型（L0–L8）与能力等级
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) —— 数据对象、Rule、TimelineEvent
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) —— ops 线上请求/响应信封
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) —— 操作库行为
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) —— connect 信封族
- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) —— 仓库概览、安装与快速开始
