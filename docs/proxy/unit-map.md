# 单元地图 · proxy

> 定位：proxy（核心代理工具）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/proxy/api.rs`

> 本文是 **proxy** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**核心代理工具的唯一落点**：六工具的载荷与回执、授权判据、幂等账与成员侧执行者都只在这里实现。
> 它**不持落盘状态、不持会话**：幂等账只在内存，真实动作经 `ProxyHost` 由 `runtime` 的 `ProxyBridge` 落回核心线程。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/proxy.rs` | 能力入口与公开面（`api` / `domain` / `service` 三个分区） |
| `src/capabilities/proxy/api.rs` | 唯一入口：`src/capabilities/proxy/domain/proxy.rs` 的值类型与纯规则经这里转出，执行面 `ProxyHandler` / `ProxyTools`，以及宿主契约 `ProxyHost`（六个动作） |
| `src/capabilities/proxy/domain.rs` | 纯形状分区入口（`proxy` 子模块） |
| `src/capabilities/proxy/service.rs` | 执行面分区入口（`proxy` 子模块） |
| `src/capabilities/proxy/domain/proxy.rs` | 六工具的载荷与回执：`*Args`、`CatalogScope` / `ObserveView` / `ControlAction` 的解析、授权判据 `authorize`、`resolve_new_session` / `relay` / 三个请求校验器 / `render_catalog` |
| `src/capabilities/proxy/service/proxy.rs` | 执行面：`ProxyTools::call` 的认领 → 参数 → 幂等 → 授权 → 宿主五步与幂等账；`ProxyHandler`（代理会话那一回合的 `ToolHandler`） |
