---
layout: home
hero:
  name: SPOKE
  text: Standardized Programmable Ontology Knowledge Engine
  tagline: 集成文档 —— 面向叙事知识的线上方言，每种任务都有一条路径。
  actions:
    - { theme: brand, text: 构建 Adapter, link: /zh/how-to/implement-adapter }
    - { theme: alt, text: 开启 connect 会话, link: /zh/tutorials/first-connect-session }
features:
  - { title: 教程, details: "两条端到端路径：安装并 upsert 你的第一条 KnowledgeEntry，然后开启你的首个 connect 会话。", link: /zh/tutorials/install-and-first-entry }
  - { title: 操作指南, details: "集成方旅程：实现你的能力所需的 adapter ports，再用 TypeScript 客户端、基于消费方 `Transport` 的 RemoteAdapter、跨多个对等节点路由或原生绑定接入跨主机 connect。", link: /zh/how-to/implement-adapter }
  - { title: 参考, details: "在站内核对线上事实：协议、数据模型、ops 与 connect 字段表，溯源到 schema。", link: /zh/reference/protocol }
  - { title: 讲解, details: "线上背后的概念：九层模型、能力标志、双重关注对与四个 Domain Profile。", link: /zh/explanation/concepts }
---

## 从这里开始

如果你的产品存储叙事知识，请[实现 Adapter](/zh/how-to/implement-adapter) —— 为你声明的能力挑选 port 族，并调用匹配的编排器。如果你运行独立的 SPOKE 主机并想与另一台主机对话，请[开启 connect 会话](/zh/tutorials/first-connect-session)，或直接跳到 [TypeScript 客户端](/zh/how-to/connect-ts-client) / [原生绑定](/zh/how-to/connect-native-bindings)。自带消息导向 `Transport`？通过 [RemoteAdapter over a Transport](/zh/how-to/connect-remote-adapter) 拨号远端主机、[跨多个对等节点路由](/zh/how-to/multi-peer-routing)，或经[原生绑定使用 RemoteAdapter](/zh/how-to/remote-adapter-native-binding) 驱动同一 adapter 面。用[远程工具](/zh/how-to/connect-remote-tools)暴露可被主机发现并在编排中途反向调用的工具。全新用户按顺序完成两个教程：[安装并创建你的第一条 KnowledgeEntry](/zh/tutorials/install-and-first-entry)，然后是[开启你的首个 connect 会话](/zh/tutorials/first-connect-session)。

## 为什么选择 SPOKE

SPOKE 是一套面向叙事知识的 JSON Schema 线上契约协议：各独立产品通过共享的数据与 ops 形状交换一致性检查和上下文组装的 I/O，避免每个产品为同一概念各自发明本地格式。本仓库即单一事实来源（SSOT）—— 手写 schema、生成的 TypeScript 与 Rust 线上类型，以及纯函数操作库。

- **单一线上方言** —— 九个数据对象与五个基线 ops（另含可选 `project` / `compute`）
- **基于开放字符串的 Domain Profile** —— 本体词汇经开放字符串发布，并经可选 `modules.*` 字段袋承载跨产品方言
- **能力标志** —— 声明 `spoke-baseline`，或显式声明 `l2-computable`、`l5-fork`、`l5-mind`、`narrative-modules`、`spoke-connect`
- **语言对齐** —— npm 与 crates.io 共用一套锁步 SemVer
- **可选 connect** —— 已签名的跨进程交互信封，带按会话排序与可扩展鉴权

## 延伸阅读

- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) —— 协议词汇，以线上术语定义
- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) —— 仓库概览、安装与快速开始
- [软件包快速开始](/zh/packages/quick-start) —— npm 与 crates.io 安装固定
- [版本与发布](/zh/release/versioning) —— 面向集成方的锁步 SemVer
