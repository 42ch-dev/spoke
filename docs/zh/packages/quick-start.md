---
title: 软件包快速开始
---

# 软件包快速开始（Package quick-start）

SPOKE 发布四个消费方软件包 —— 生成的线上类型与手写操作库 —— 共用一套**锁步 SemVer**。将您使用的所有面固定到同一个 `X.Y.Z`。

本页为[英文原页](/packages/quick-start)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## TypeScript（npm）

```bash
pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z
```

- **`@42ch/spoke-schemas`** —— 生成的线上类型（从包根导入 `KnowledgeEntry`（知识条目）、`TimelineEvent`（时间轴事件）、`PromoteRequest`、`AssemblePacket`（上下文组装载荷）、`HostCapabilityManifest`（主机能力清单）等）。
- **`@42ch/spoke-operations`** —— 纯辅助函数、按能力切分的 adapter ports 与 `orchestrate*` 入口。

## Rust（crates.io）

```bash
cargo add spoke-schemas@X.Y.Z spoke-operations@X.Y.Z
```

```toml
[dependencies]
spoke-schemas = "X.Y.Z"
spoke-operations = "X.Y.Z"
```

- **`spoke-schemas`** —— 由同一 JSON Schema SSOT 生成的 Rust 线上类型。
- **`spoke-operations`** —— port traits 与 `orchestrate_*`（重新导出 `spoke_schemas`）。

## 集成路径

1. 从 schemas 软件包导入线上类型。
2. 在一个 adapter 类型上为所声明能力实现 port 族（`BaselineAdapter` … `FullAdapter`）。
3. 调用匹配的编排器（`orchestrateUpsert`、`orchestratePromote` 等）—— 纯门禁运行，持久化经您的 ports 进行。
4. 走一遍已提交的 "Mira at Harbor" 图与 `fixtures/toy-world/` 中的参考 `ToyWorldAdapter`（TypeScript adapter 与 Rust fixture crate）。

## 版本策略

全部软件包一起升版（锁步 SemVer）；带注释标签 `vX.Y.Z` 与之匹配。固定指南见 [版本与发布](/zh/release/versioning)。

## 规范参考

- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) —— 安装、快速开始、操作概览
- [@42ch/spoke-schemas README](https://github.com/42ch-dev/spoke/blob/main/packages/spoke-schemas/README.md)
- [@42ch/spoke-operations README](https://github.com/42ch-dev/spoke/blob/main/packages/spoke-operations/README.md)
- [spoke-schemas README (Rust)](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-schemas/README.md)
- [spoke-operations README (Rust)](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-operations/README.md)
- [spoke-version-release.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-version-release.md) —— 锁步 SemVer 与消费方固定
