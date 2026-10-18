# residents（常驻服务）

> 模块声明的常驻外部服务（按行 JSON-RPC 一类，当前 MCP stdio）的统一管理：拉起、发现操作、调用、停止、按租约回收。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：常驻服务的注册表与运行态（启动 / 停止、崩溃检测、租约回收）、操作发现与调用的统一入口（`api::ResidentOps`）、协议适配器端口（`ports::ServiceAdapter`）。

**不管**：任何具体协议（当前 MCP stdio 在 `detail/mcp.rs`，只由组合根构造）；进程与围栏机制（在 kernel）；**启停有效值与落盘（归 `modules`）**；模块声明与契约（在 `modules`）。

## 二、入站契约与状态归属

`api`（`ResidentOps` + `ServiceView` / `Operation` / `ServiceState` / `Receipt` + 空实现 `NoResidents`）；出站端口 `ServiceAdapter` **只由 `service.rs` 持有**（R12）。
状态在 `service.rs`：运行中的实例（key = 模块/服务）、**失败态**（`call` 发现进程已结束 → `Failed`，带如实原因与租约）；会话租约用于回收（`reap`）。启停的**有效值**由 `modules` 给（用户覆盖，否则作者默认；模块停用 ⇒ 它的服务停用），启动前先查它。
**崩溃检测**：适配器按连接事实回答 `ServiceInstance::is_alive`；下一次 `call` 发现会话已结束（EOF）时，如实标 `Failed`（原因「服务进程已结束」+ 租约）并从运行态摘除。
**不可用的回执**：`call` 先按 `modules` 的有效值判可用性，再按运行态给**真实原因**——`模块 <m> 已停用` / `已停用（需先启用）` / `进程已结束（上次：…）` / `未启动`。回执交给模型（工具循环把它当工具结果），不靠会话侧缓存或刷新。
**操作清单变化**：服务报告 `notifications/tools/list_changed` 时适配器交回新清单（`take_refreshed_operations`），运行态就地刷新，下一次装配的工具面随之反映。
**可见面**：就绪服务的操作进动作目录（`GET /api/actions` 对 user 可见）与模型工具面（原生声明 + 信封清单）；CLI `resident list|start|stop|enable|disable|call` 是同一批动作的终端入口。
**参数契约**：`Operation.parameters` 是服务给的 **JSON Schema 原样**（MCP 的 `inputSchema`）；核心**不解释、不校验**它——原生通道原样发进工具声明，信封通道渲染成参数说明（名字 / 类型 / 必填 / 缺省 / 可选值 / 说明）。参数错了由服务如实回绝（`isError`）。

## 三、当前能力与适配器约束

- 生命周期动作 `control_resident`（只给 user）：start / stop / enable / disable。
- 会话删除时按租约回收（`reap`）；失败态也按租约清掉。
- 起服务时按 `secrets::resolve` 注入 env；回执经 `secrets::redact` 脱敏。
- 起服务时的围栏与「人直接跑模块工具」同一条口径：模块根递归只读、`userdata/` 可写，并挂上本次实例的会话租约（`lease`）。
- 模块动作 `module.<id>.<service>.<op>` 转给 `ResidentOps::call`。
- MCP stdio 适配器（`detail::McpAdapter`）：握手 / `tools/list` / `tools/call`；长驻会话走 kernel 的 `SessionHost`。
- 父死子亡：`SessionHost::open` 走 `procs::guard` → `spawn` → `attach`（Linux `PR_SET_PDEATHSIG` / macOS `--proc-watchdog` / Windows 作业对象）；核心被强杀不留服务进程。守门进程登记进进程账（`purpose = resident`，归属 `resident/<模块>/<服务>`）：会话收掉时注销，挂不上父死子亡时如实记进账，孤儿由启动对账按身份收掉。机制见 [kernel 的「长驻子进程的账与回收」](../kernel/README.md)。
- 模型侧工具面：已就绪服务的操作以 `<服务>.<操作>` 进成员工具面，**参数契约原样声明**给模型，调用路由到 `ResidentOps::call`。
- 崩溃检测与 `tools/list_changed` 刷新。
- 适配器约束：`services.<名>.adapter` 只认组合根注册过的适配器（当前 `mcp`）；未知适配器在服务视图里如实报「未知适配器」并拒收启动。其它协议的适配器属于缺口，见 `tests/gaps.yaml` 的 `module.acp-adapter`。

## 四、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`（长驻进程机制）、`modules`（取模块清单事实）、`secrets`（起进程时解析隐秘字段）
- 谁在用我：`collab`、`conductor`、`session`（`Ops` 字段与成员工具面，由组合根注入）

## 五、改动本单元时必须同步

- 新增/变更适配器端口 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵；真实适配器（当前 `mcp`）在 `detail/`，只由组合根构造。
- 模块的 `services:` 声明契约见 [MODULE_SPEC.md](../../MODULE_SPEC.md)；声明解析在 `workspace`。
- 真机演示与回归载体：`modules/mcp-toolbox` + `demo/run-demo-mcp.mjs`（真实模型、真机）；
  `tests/cross-platform/e2e/` 的 mcp 场景（夹具 + Mock 供应商，进 CI）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
