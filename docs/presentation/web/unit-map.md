# 单元地图 · presentation/web

> 本文是 **presentation/web** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/presentation/web/mod.rs` | 本地 HTTP 服务（tiny_http，只绑 `127.0.0.1`）与长轮询；按 `ROUTES` 匹配与分发 |
| `src/presentation/web/routes.rs` | HTTP 路由目录 `ROUTES` 与匹配器：路由的唯一定义，`--print-routes` 输出它的 JSON |
| `src/presentation/web/assets/` | 浏览器端资源（`index.html` / `tokens.css` / `ui.css` / `ui.js` / `style.css` / `app.js` / `md.js`）与冒烟脚本（`*.smoke.cjs`） |
