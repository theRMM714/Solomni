# 模块地图 · conductor

> 本文是 **conductor** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张模块地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：会话在世表、命令队列、运行态与跨会话编排。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conductor/mod.rs` |
| `src/capabilities/conductor/api/mod.rs` |
| `src/capabilities/conductor/api/handle.rs` |
| `src/capabilities/conductor/api/proxy.rs` |
| `src/capabilities/conductor/service/mod.rs` |
| `src/capabilities/conductor/service/work.rs` |
| `src/capabilities/conductor/service/turn.rs` |
| `src/capabilities/conductor/service/flow.rs` |
| `src/capabilities/conductor/service/rewind.rs` |
| `src/capabilities/conductor/service/env.rs` |
| `src/capabilities/conductor/service/history.rs` |
