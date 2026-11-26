# 单元地图 · residents

> 定位：residents（常驻服务）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/residents/api.rs`

> 本文是 **residents** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/residents/api.rs` | 统一管理 API（`ResidentOps`）与 DTO；未注入时的空实现 `NoResidents` |
| `src/units/business/residents/ports.rs` | 协议适配器端口（`ServiceAdapter` / `ServiceInstance` / `LaunchSpec`） |
| `src/units/business/residents/service.rs` | 注册表与生命周期（启停 / 开关 / 调用 / 租约回收） |
| `src/units/business/residents/detail.rs` | 机制实现入口（只有组合根能构造） |
| `src/units/business/residents/detail/mcp.rs` | MCP stdio 适配器（握手 / tools/list / tools/call） |
