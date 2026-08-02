---
title: 版本与发布
---

# 版本与发布（Version & release）

SPOKE 在所有 workspace 软件包与 Rust 消费方 crate 上发布**单一锁步 SemVer**，集成方在 TypeScript 与 Rust 产物间只固定一个版本。发布物为带注释的 git 标签 `vX.Y.Z`，物化为 GitHub Releases；每个不含 `-rc.` 预发布段的标签都会发布到 npm 与 crates.io。

本页为[英文原页](/release/versioning)（English）的中文概览。规范说明（specs）保持英文原文并作为 SSOT（单一事实来源）；完整细节见文末「规范参考」。

## 固定什么

- **npm** —— `@42ch/spoke-schemas`、`@42ch/spoke-operations`
- **crates.io** —— `spoke-schemas`、`spoke-operations`
- 各处使用**同一** `X.Y.Z`；标签 `vX.Y.Z` 与锁步版本匹配。

## 发布形态

- **锁步清单** —— workspace 根、两个 TS 软件包、两个 Rust crate、fixture/codegen 工具与 `Cargo.lock` 共享一个版本；CI 断言零漂移（`pnpm run verify:version`）。
- **标签** —— 带注释的 `vX.Y.Z`。每个不含 `-rc.` 的标签 —— 稳定版 `vX.Y.Z` 或预发布版如 `v0.1.0-alpha.3` —— 都会发布到 npm 与 crates.io；含 `-rc.` 的标签（`vX.Y.Z-rc.N`）仅创建 GitHub 预发布。
- **发布说明** —— 来自 `CHANGELOG.md`，由 git-cliff 依据 Conventional Commits 生成。
- **发布** —— 每个不含 `-rc.` 的标签经 CI Trusted Publishing 发布到 npm 与 crates.io；`-rc.` 标签仅创建 GitHub 预发布。

## SemVer 语义

- **PATCH** —— 仅缺陷修复的打包发布；线上契约保持不变。
- **MINOR** —— 向后兼容的线上或操作库新增。
- **MAJOR** —— 破坏性的线上或公开 API 变更（1.0 之前，允许不经弃用期）。

## 固定方法

- npm：`pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z`
- crates.io：`spoke-schemas = "X.Y.Z"`、`spoke-operations = "X.Y.Z"`
- Git 标签：`git checkout vX.Y.Z`；或使用该标签的 GitHub Release 源码归档。

## 软件包 SemVer 与线上 `schema_version`

软件包 SemVer 与整数线上 `schema_version` 独立变动 —— 一方的升版不要求另一方升版。作为例外，当一次发布同时覆盖软件包升版与线上变更时，发布说明会显式耦合两者。

## 规范参考

- [spoke-version-release.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-version-release.md) —— 锁步面、标签约定、CI 要求、消费方固定
- [CHANGELOG.md](https://github.com/42ch-dev/spoke/blob/main/CHANGELOG.md) —— 发布说明
- [GitHub Releases](https://github.com/42ch-dev/spoke/releases) —— 发布物与源码归档
- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) —— 安装与固定示例
