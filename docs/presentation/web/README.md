# web（本地网页）

> 本地 HTTP + 长轮询；路由目录是机器比对的锚点。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：本地 HTTP（tiny_http，**只绑 `127.0.0.1`**）+ 长轮询增量推送 + HTTP 路由目录 + 浏览器端资源与冒烟（`assets/*.smoke.cjs`）。浏览器端覆盖三种工作形态（single / collab / **proxy**）的建工作向导与会话视图：代理形态不问名单与需求（选它就是授予全权），会话说明与侧栏标出"已停止 / 已关闭"（持久运行态）。
另外管**依赖补齐的界面入口**：虚拟机档不可用时，配置面板与创建向导给出「补齐虚拟机依赖」（看清单与体积 → 确认 → 按 `seq` 长轮询进度 → 如实说清生效范围）；
要装哪些条目由后端 `/api/setup/status` 的 `pending` 决定，界面不自己挑。
基本设置弹层还列**现在在跑什么**（`GET /api/processes`：产品起的 QEMU / 共享进程 / 常驻服务守门进程与它们的归属；`alive=false` 如实标成「已退出，等对账清」，`orphanRisk` 标出父死子亡没挂上、核心被强杀会留下的那条）。

**不管**：不做业务判断、不持状态、不落盘；**永不接触端口对象，也拿不到 `Core` 本身**。

## 二、入站契约与状态归属

`src/presentation/web/routes.rs` 的 `ROUTES` 是 HTTP 入站契约的**唯一定义**，与 `docs/presentation/contracts.md` 的路由表机器比对（`src/tests/routes.rs`）。

入口：`src/main.rs` 调 `presentation::web::serve(ops, port, fence)`；默认端口是 `presentation::web::DEFAULT_PORT`（`webui` 与启动参数共用这一个来源）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`conductor`（`Ops` 字段里带 `provision` 面）
- 谁在用我：（无）

## 四、改动本单元时必须同步

- 增删路由 → 五处同改：`src/presentation/web/routes.rs` 的 `ROUTES`、前端调用点、`docs/presentation/contracts.md` 的 ROUTES 段、`src/tests/routes.rs`（机器比对）、`demo/*.mjs` 的调用点；前端资源改动跑冒烟。
- 测试缺口：本单元**不是业务能力**，没有独立的缺口账；缺口记在 `tests/gaps.yaml`；格式见 [docs/testing/gaps-acceptance.md](../../../docs/testing/gaps-acceptance.md) §十二。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
