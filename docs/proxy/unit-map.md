# 单元地图 · proxy

> 定位：proxy（工具面业务）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/proxy/api.rs`

> 本文是 **proxy** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**模型工具面的唯一入口**（§九.1 路径 C）：动作跨会话 / 工作区 / 工具 / 权限多个参与方，
> 无法归属其中任一方；它不持落盘状态，外部动作一律经端口交回核心线程。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/proxy.rs` | 能力入口与公开面 |
| `src/capabilities/proxy/api.rs` | 入站契约：`ProxyHost`、`ProxyTools` / `ProxyHandler` 与 `domain::proxy` 的类型重导出 |
| `src/capabilities/proxy/ports.rs` | 回调端口：核心代理工具要做的外部动作（`ProxyHost`，由宿主实现） |
| `src/capabilities/proxy/domain/proxy.rs` | 纯逻辑：入参解析、授权校验、载荷规整与回执拼装 |
| `src/capabilities/proxy/service/proxy.rs` | 执行面：`ProxyTools` 的六个工具与成员侧执行者 `ProxyHandler` |
