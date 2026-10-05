# slate（名单）

> 按需求提出名单并收束；无状态、无端口。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：按需求提出名单并按模式收束：协作 = 原样；单 agent = 最多一条，多条并成一个临时 agent（模块去重、模型取首项、理由合并）。

**不管**：**没有端口、没有状态**——名单是在飞的值（归提出方）；参与方事实由调用方传入。

## 二、入站契约与状态归属

`api::propose` + DTO（`Mode` / `Pick` / `Proposal` / `Parties` / `Request`）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`llm`、`prompt`、`registry`、`session`、`tools`、`workspace`
- 谁在用我：`collab`、`conductor`

## 四、改动本单元时必须同步

- 收束规则一改 → `src/tests/slate.rs` 与两个调用方（`conductor` 的一次性推荐、`collab` 的代拟）的用例。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |