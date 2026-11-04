# collab（协作会话）

> 定位：从讨论走到交付的协作会话状态机。
> 效力：规范｜核对：2026-11-04｜依据：`src/capabilities/collab/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：协作会话对象与状态（名单、方案、任务链、挂起）、讨论泵与轮转发言、成员回合（含工具循环）、整理出链、链驱动、节点与总验收及定向返工、断点续跑。
- 不管：不落盘（经 `session::api`）、不建会话（`conductor` 建）、不选模型（`registry` 解析）、不画界面（`cli` / `web`）。

## 入站契约与状态归属

- 入站面：`api::CollabSession`——讨论与执行引擎、回合与验收词汇、协作状态派生的对外名字。
- 状态归属：会话对象及其状态只有它写；「转录即状态」的派生在 `src/capabilities/collab/domain/collab_state.rs`（纯函数、可回放）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`modules`、`prompt`、`registry`、`residents`、`session`、`slate`、`taskchain`、`tools`、`vm`、`workspace`
- 谁在用我：`conductor`

## 改动本单元时必须同步

- 状态派生（`src/capabilities/collab/domain/`）与驱动（`src/capabilities/collab/service/driver.rs`）必须同改；同步 `src/tests/collab/` 五份用例、`docs/collab/task-chain.md`、`docs/session/session-model.md`、`docs/tools/tools-and-roles.md`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`task-chain.md`](task-chain.md) | 审查关卡、任务链（依赖图）、子会话与验收 |
