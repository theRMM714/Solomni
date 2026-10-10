# 单元地图 · kernel

> 本文是 **kernel** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> **无领域语义、无领域状态**的机制；形状与别的业务一致（`api` / `ports` / `domain` / `detail`），但它在依赖图的最底层：**不认识任何能力**。
> `Log` / `HostProbe` / `ToolHandler` / `AskUser` / `ProcessRunner` / `ProcessTexts` / `FenceHost` 是**全项目共享**的机制端口（R12 的例外）。

| 文件 | 职责 |
| --- | --- |
| `src/kernel/mod.rs` |
| `src/kernel/api.rs` |
| `src/kernel/ports.rs` |
| `src/kernel/domain/types.rs` |
| `src/kernel/domain/fence.rs` | 围栏描述符（`FenceSpec`）与落点判据（`FencePart`/`FenceBlocked`/`fence_ask`）：纯数据 |
| `src/kernel/domain/path.rs` |
| `src/kernel/domain/jobs.rs` |
| `src/kernel/detail/file_log.rs` |
| `src/kernel/detail/host_probe.rs` |
| `src/kernel/detail/process.rs` | 外部进程执行机制（守门进程、stdin 送参、隐私字段 env 注入、超时杀树、截断），实现 `ProcessRunner` |
| `src/kernel/detail/confine/mod.rs` | 围栏机制入口（策略由调用方传入；实现 `FenceHost` 释放） |
| `src/kernel/detail/confine/linux.rs` |
| `src/kernel/detail/confine/macos.rs` |
| `src/kernel/detail/confine/other.rs` |
| `src/kernel/detail/confine/windows/` |
| `src/kernel/detail/confine/windows/mod.rs` |
| `src/kernel/detail/confine/windows/acl.rs` |
| `src/kernel/detail/confine/windows/record.rs` |
| `src/kernel/detail/confine/windows/container.rs` |
| `src/kernel/detail/confine/windows/tests.rs` |
