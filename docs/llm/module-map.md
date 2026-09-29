# 模块地图 · llm

> 本文是 **llm** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张模块地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/llm/api.rs` |
| `src/capabilities/llm/service.rs` |
| `src/capabilities/llm/ports.rs` |
| `src/capabilities/llm/detail/http_chat.rs` |
| `src/capabilities/llm/detail/http_probe.rs` |
| `src/capabilities/llm/detail/repair.rs` |
| `src/capabilities/llm/detail/http_agent.rs` |
| `src/capabilities/llm/detail/endpoint.rs` |
| `src/capabilities/llm/detail/model_catalog.rs` |
| `src/capabilities/llm/detail/fake_chat.rs` |
| `src/capabilities/llm/domain/envelope.rs` |
| `src/capabilities/llm/domain/malformed.rs` |
