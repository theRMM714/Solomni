# 单元地图 · presentation/web

> 定位：presentation/web（本地 HTTP 呈现）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-10｜依据：`src/presentation/web/mod.rs`

> 本文是 **presentation/web** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/presentation/web/mod.rs` | 本地 HTTP 服务（tiny_http，只绑 `127.0.0.1`）与长轮询；按 `ROUTES` 匹配与分发 |
| `src/presentation/web/routes.rs` | HTTP 路由目录 `ROUTES` 与匹配器（末段 `{*name}` 为尾部通配）：路由的唯一定义，`--print-routes` 输出它的 JSON |
| `src/presentation/web/assets.rs` | 浏览器端资源的嵌入表（`build.rs` 生成）与按 URL 路径精确查找；不读文件系统 |
| `build.rs` | 编译期扫描 `src/presentation/web/assets/` 生成嵌入表：按扩展名定 MIME，跳过 `.cjs` / `.mjs` 开发脚本与 `package.json`，未知扩展名让构建失败；`DEV_ONLY_DIRS`（`dev/`）只在 `PROFILE=debug` 时收进表，随 `rerun-if-env-changed=PROFILE` 重跑 |
| `src/presentation/web/assets/` | 浏览器端资源（`index.html` 只放挂载点、`tokens.css` / `ui.css` / `style.css`、`md.js`，经 `/assets/{*path}` 提供）与冒烟脚本（`*.smoke.cjs` / `*.smoke.mjs`，`smoke.cjs` 递归发现；`tokens.smoke.cjs` 判令牌与对比度，`copy.smoke.cjs` 判文案分层） |
| `src/presentation/web/assets/dev/` | 只在 debug 构建嵌入的开发工具：`gallery.html` / `gallery.js` 陈列令牌、组件与图标三态，`gallery.smoke.cjs` 校验它引用真实组件、不写死颜色 |
| `src/presentation/web/assets/app/` | 浏览器端 ES Module 的唯一入口 `main.js`（装配点：建共享上下文、按顺序 `mount`、启动接线）；`package.json` 只给 node 声明模块类型 |
| `src/presentation/web/assets/app/core/` | 不依赖其它层的原语：`http.js` HTTP 封装、`events.js` 长轮询传输、`dom.js` 的 DOM 原语、`format.js` 纯格式化函数 |
| `src/presentation/web/assets/app/stores/` | API 回包与事件的投影（只由回包与事件写入）：`state.js` 快照、`sessions.js` 会话读法、`settings.js` 思考档位与权限文案 |
| `src/presentation/web/assets/app/ui/` | 组件库（只收参数、发回调，只依赖 `core/`）：`index.js` 组装，`icons` / `buttons` / `forms` / `overlays`（含 `detail` 三段文案）/ `dialog` / `modal` |
| `src/presentation/web/assets/app/features/` | 各功能区（每个导出 `mount(root, deps)`，功能之间互不 import）：`sidebar` / `wizard` / `session-config` / `tabs` / `status-bar` / `composer` / `transcript` / `actions` / `work` / `shell` / `settings` / `agents` / `files` / `setup` / `modules` |
