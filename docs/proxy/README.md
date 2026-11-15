# proxy（工具面业务）

> 定位：模型工具面 `core_proxy` 的入口——六个工具的声明解读、授权校验、参数规整与执行。
> 效力：规范｜核对：2026-11-15｜依据：`src/capabilities/proxy/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：`core_proxy` 六个工具的**执行面**——工具 id 与角色面判定、任务级授权校验（`Grant`）、参数语义校验与规整载荷、回执拼装。
- 管：执行流程的三道闸——按声明校验参数 → 判授权 → 查幂等账；随后把动作交给宿主端口 `ProxyHost`。
- 管：成员侧执行者 `ProxyHandler`——装进代理会话的工具环境后，代理会话就是一条普通成员会话。
- 不管：会话机制本身——清单、建会话、转达、观察、生命周期与消息倒查由 `conductor` 的队列桥落到核心线程（`ProxyBridge`）。
- 不管：工具面名单与 callers（在 `systools/tools.yaml`）；权限策略与围栏（在 `permission` / `tools`）。

## 入站契约与状态归属

- 入站面：`api::ProxyHost`（**回调端口**：proxy 声明、宿主实现，见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §二）、`api::ProxyTools`（执行器）、`api::ProxyHandler`（成员侧执行者，实现 `kernel` 的 `ToolHandler`），以及 `domain::proxy` 的全部公开类型（`ProxyCall`、`Grant`、`Catalog`、`Relayed` 等）。别的业务与测试只经 `api` 取用（R1）。
- 状态归属：**不持任何落盘状态**——幂等账是执行器内存里的 `BTreeMap`，随会话生命周期消失；授予读会话 `meta.delegation`，转录归 `session`，工作区归 `workspace`。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`permission`、`registry`、`session`、`tools`
- 谁在用我：`conductor`

## 一、执行面的三道闸

- 参数先过工具声明：`tools` 的 `ToolBook` 查不到、必填缺项或多余键一律拒绝，错误文案由 `domain` 拼装（`src/capabilities/proxy/service/proxy.rs`）。
- 授权按任务级 `Grant` 判：工具不在授予名单里就拒绝，不落到宿主动作（`src/capabilities/proxy/domain/proxy.rs`）。
- 建会话 / 转达 / 生命周期按 `request_id` 记账：同一请求重放回同一结果，不再动宿主；多目标转达允许部分成功（`sent` / `failed`）。

## 改动本单元时必须同步

- `src/capabilities/proxy/ports.rs` → 端口方法变化必须同改宿主实现（`conductor` 的 `ProxyBridge`）与测试替身 `src/tests/doubles/proxy.rs`。
- `systools/tools.yaml` → 工具面名单或入参形状变化时，`src/capabilities/proxy/domain/proxy.rs` 的解读与 `src/tests/conductor/proxy.rs` 的用例一起改。
- `src/capabilities/proxy/api.rs` → 新增入站方法时核对 `docs/conductor/unit-map.md` 的装配行与本页的「入站契约」。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
