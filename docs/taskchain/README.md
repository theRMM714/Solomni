# taskchain（任务链）

> 纯领域：依赖图、阶段派生、就绪与验收判定。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：任务链的事实与派生——节点、依赖、阶段派生（最长路径分层）、就绪、验收判定与装配错误上报（含环拒绝）。

**不管**：**纯领域业务**：没有端口、没有 `service`；不做 IO、不碰会话、不认识 LLM。

## 二、入站契约与状态归属

`api`（`TaskChain` / `TaskNode` / `NodeStatus` / `Acceptance` + 阶段 / 就绪 / 验收判定）。三个消费者：`collab` 驱动、`session` 线格式携带、`web` 渲染。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——依赖图最底层或纯领域）
- 谁在用我：`collab`、`conductor`、`session`

## 四、改动本单元时必须同步

- 图算法一改 → `src/tests/taskchain.rs`；驱动语义与验收流程见 `docs/collab/task-chain.md`；线格式改动连带 `session`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |