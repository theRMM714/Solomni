# 模块地图 · kernel

> 本文是 **kernel** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张模块地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> **无领域语义、无领域状态**的机制；形状与别的业务一致（`api` / `ports` / `domain` / `detail`），但它在依赖图的最底层：**不认识任何能力**。
> `Log` / `HostProbe` 是**全项目共享**的机制端口（R12 的例外）：谁都可以持有它们。

| 文件 | 职责 |
| --- | --- |
| `src/kernel/mod.rs` |
| `src/kernel/api.rs` |
| `src/kernel/ports.rs` |
| `src/kernel/domain/types.rs` |
| `src/kernel/domain/path.rs` |
| `src/kernel/domain/jobs.rs` |
| `src/kernel/domain/approvals.rs` |
| `src/kernel/detail/file_log.rs` |
| `src/kernel/detail/host_probe.rs` |
