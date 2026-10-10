# 单元地图 · tools

> 本文是 **tools** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/tools/api.rs` |
| `src/capabilities/tools/service/mod.rs` |
| `src/capabilities/tools/ports.rs` |
| `src/capabilities/tools/service/systool.rs` |
| `src/capabilities/tools/domain/systool.rs` |
| `src/capabilities/tools/domain/patch.rs` |
| `src/capabilities/tools/domain/schema.rs` |
| `src/capabilities/tools/domain/module_tools.rs` |
| `src/capabilities/tools/domain/roles.rs` |
| `src/capabilities/tools/domain/fence.rs` | 围栏描述符的派生策略（其余中立类型在 `kernel/domain/fence.rs`） |
| `src/capabilities/tools/detail/prompt_process_texts.rs` | 提示词册 → kernel `ProcessTexts` 端口的适配器 |
| `src/capabilities/tools/detail/sys_io.rs` |
| `src/capabilities/tools/detail/yaml_systools.rs` |
