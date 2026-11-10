# 单元地图 · presentation/web

> 定位：presentation/web（本地 HTTP 呈现）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-04｜依据：`src/presentation/web/mod.rs`

> 本文是 **presentation/web** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/presentation/web/mod.rs` | 本地 HTTP 服务（tiny_http，只绑 `127.0.0.1`）与长轮询；按 `ROUTES` 匹配与分发 |
| `src/presentation/web/routes.rs` | HTTP 路由目录 `ROUTES` 与匹配器（末段 `{*name}` 为尾部通配）：路由的唯一定义，`--print-routes` 输出它的 JSON |
| `src/presentation/web/assets.rs` | 浏览器端资源的嵌入表（`build.rs` 生成）与按 URL 路径精确查找；不读文件系统 |
| `build.rs` | 编译期扫描 `src/presentation/web/assets/` 生成嵌入表：按扩展名定 MIME，跳过 `.cjs` / `.mjs` 开发脚本与 `package.json`，未知扩展名让构建失败 |
| `src/presentation/web/assets/` | 浏览器端资源（`index.html` / `tokens.css` / `ui.css` / `style.css` / `app.js` / `md.js`，经 `/assets/{*path}` 提供）与冒烟脚本（`*.smoke.cjs` / `*.smoke.mjs`，`smoke.cjs` 递归发现） |
| `src/presentation/web/assets/app/` | 浏览器端 ES Module：`main.js` 入口；`core/`（`http.js` HTTP 封装、`events.js` 长轮询传输）；`stores/`（`state.js` 快照投影与登记处通知）；`ui/`（组件库：`index.js` 组装，图标 / 按钮 / 表单 / 浮层各一个文件，只收参数、发回调）；DOM 原语与 `h()` 在 `src/presentation/web/assets/app/core/dom.js`；`bridge.js` 把它们交给经典脚本 `app.js`；`package.json` 只给 node 声明模块类型 |
