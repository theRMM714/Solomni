# residents（常驻服务）

> 模块声明的常驻外部服务（MCP / harness 等）的统一管理：拉起、发现操作、调用、停止、按租约回收。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：常驻服务的注册表与生命周期（启停、开关、租约回收）、操作发现与调用的统一入口（`api::ResidentOps`）、协议适配器端口（`ports::ServiceAdapter`）。

**不管**：任何具体协议（MCP / ACP 全在适配器实现里，只由组合根构造）；进程与围栏机制（在 kernel）；开关落盘（随设置面接入）；模块声明的解析（在 `workspace`）。

## 二、入站契约与状态归属

`api`（`ResidentOps` + `ServiceView` / `Operation` / `ServiceState` / `Receipt` + 空实现 `NoResidents`）；出站端口 `ServiceAdapter` **只由 `service.rs` 持有**（R12）。
状态在 `service.rs`：运行中的实例（key = 模块/服务）、**失败态**（起过但进程已结束 → `Failed`，带如实原因与租约）与用户开关；会话租约用于回收（`reap`）。
**崩溃检测**：适配器按连接事实回答 `ServiceInstance::is_alive`；进程一结束，`call` 如实标 `Failed` 并从运行态摘除，后续调用如实报"没在跑"。
**操作清单变化**：服务报告 `notifications/tools/list_changed` 时适配器交回新清单（`take_refreshed_operations`），运行态就地刷新，下一次装配的工具面随之反映。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`workspace`（取模块清单事实）
- 谁在用我：`conductor`（`Ops` 字段；Phase 1 由组合根注入）

## 四、改动本单元时必须同步

- 新增/变更适配器端口 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵；真实适配器（MCP/ACP）在 `detail/`，只由组合根构造。
- 模块的 `services:` 声明契约见 [MODULE_SPEC.md](../../MODULE_SPEC.md)；声明解析在 `workspace`。
- 动作面（`control_resident`）、会话删除时的租约回收、起服务时按 `secrets::resolve` 注入 env 与回执脱敏、`ResidentOps::call`（经模块动作 `module.<id>.<service>.<op>`）、**MCP stdio 适配器**（`detail::McpAdapter`，长驻会话走 kernel 的 `SessionHost`）**都已接入**；**模型侧**工具面也已接入——已就绪服务的操作以 `<服务>.<操作>` 进成员工具面（原生声明与信封清单），调用路由到 `ResidentOps::call`；崩溃检测与 MCP `tools/list_changed` 刷新也已接入。ACP 适配器尚未接入，按逐项 `#[allow(dead_code)]` 如实标注。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
