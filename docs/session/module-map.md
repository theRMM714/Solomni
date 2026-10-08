# 模块地图 · session

> 本文是 **session** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张模块地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/session/detail/fs_history.rs` |
| `src/capabilities/session/api.rs` |
| `src/capabilities/session/service.rs` |
| `src/capabilities/session/ports.rs` |
| `src/capabilities/session/domain/decisions.rs` |
| `src/capabilities/session/domain/session.rs` |
| `src/capabilities/session/domain/tools.rs` |
| `src/capabilities/session/domain/history.rs` |
| `src/capabilities/session/domain/events.rs` |
| `src/capabilities/session/domain/rewind.rs` |
