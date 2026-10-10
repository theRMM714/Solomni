# 单元地图 · workspace

> 本文是 **workspace** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/workspace/api.rs` |
| `src/capabilities/workspace/service.rs` |
| `src/capabilities/workspace/ports.rs` |
| `src/capabilities/workspace/domain/module.rs` |
| `src/capabilities/workspace/domain/packages.rs` |
| `src/capabilities/workspace/domain/exec.rs` |
| `src/capabilities/workspace/domain/workspace.rs` |
| `src/capabilities/workspace/domain/hash.rs` |
| `src/capabilities/workspace/domain/workstore.rs` |
| `src/capabilities/workspace/detail/fs_modules.rs` |
| `src/capabilities/workspace/detail/fs_packages.rs` |
| `src/capabilities/workspace/detail/fs_workspace.rs` |
| `src/capabilities/workspace/detail/fs_workstore.rs` |
