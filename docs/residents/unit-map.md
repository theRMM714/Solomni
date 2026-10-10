# 单元地图 · residents

> 本文是 **residents** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/residents/api.rs` | 统一管理 API（`ResidentOps`）与 DTO；未注入时的空实现 `NoResidents` |
| `src/capabilities/residents/ports.rs` | 协议适配器端口（`ServiceAdapter` / `ServiceInstance` / `LaunchSpec`） |
| `src/capabilities/residents/service.rs` | 注册表与生命周期（启停 / 开关 / 调用 / 租约回收） |
