# 模块地图 · prompt

> 本文是 **prompt** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张模块地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/prompt/api.rs` |
| `src/capabilities/prompt/service.rs` |
| `src/capabilities/prompt/ports.rs` |
| `src/capabilities/prompt/domain/prompt.rs` |
| `src/capabilities/prompt/domain/refs.rs` |
| `src/capabilities/prompt/detail/yaml_prompts.rs` |
