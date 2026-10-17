# 单元地图 · taskchain

> 本文是 **taskchain** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/taskchain/api.rs` | 入站能力面：`TaskChain` / `TaskNode` / `NodeStatus` / `Acceptance` 与阶段、就绪、验收判定的重导出 |
| `src/capabilities/taskchain/domain/chain.rs` | 依赖图与纯函数：阶段派生（最长路径分层）、就绪、验收判定与环拒绝 |
