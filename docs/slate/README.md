# slate（名单）

> 定位：按需求提出名单并收束；无状态、无端口。
> 效力：规范｜核对：2026-11-04｜依据：`src/capabilities/slate/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：按需求提出名单并按模式收束——协作 = 原样；单 agent = 最多一条，多条并成一个临时 agent（模块去重、模型取首项、理由合并、人设为空——多份人设无法诚实合并）。
- 不管：**没有端口、没有状态**——名单是在飞的值（归提出方）；参与方事实由调用方传入。

## 入站契约与状态归属

- 入站面：`api::propose` + DTO（`Mode` / `Pick` / `Proposal` / `Parties` / `Request`）。
- 状态归属：不持有状态；名单与参与方事实都由调用方传入、调用方持有。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`llm`、`modules`、`prompt`、`registry`、`session`、`tools`
- 谁在用我：`collab`、`conductor`

## 改动本单元时必须同步

- 收束规则一改 → `src/tests/slate.rs` 与两个调用方（`conductor` 的一次性推荐、`collab` 的代拟）的用例。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
