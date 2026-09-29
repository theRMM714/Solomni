# 模块地图 · collab

> 本文是 **collab** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张模块地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/collab/api.rs` |
| `src/capabilities/collab/service/collab.rs` |
| `src/capabilities/collab/service/pump.rs` |
| `src/capabilities/collab/service/turn_io.rs` |
| `src/capabilities/collab/service/slate.rs` |
| `src/capabilities/collab/service/discussion.rs` |
| `src/capabilities/collab/service/synthesis.rs` |
| `src/capabilities/collab/service/round.rs` |
| `src/capabilities/collab/service/tool_loop.rs` |
| `src/capabilities/collab/service/driver.rs` |
| `src/capabilities/collab/domain/collab_state.rs` |
