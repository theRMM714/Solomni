# conductor（协调业务）

> 跨参与方的状态与编排：会话中心、生成驱动、`Ops` 的组装。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：会话中心（会话在世表、命令队列、运行态）、**工作形态**（single / collab / **proxy**：proxy 没有名单，用户选它就是授予全权，见 §二）、生成驱动（单 agent 与协作的长步骤都在工作线程跑，队列只占「取/交」两步）、跨能力用例与跨会话回档编排、把各能力的能力接口组装成呈现层唯一入口 `Ops`。运行态分两层：**短暂**的「这一刻谁在干活」（`SessionEvent::Working`，不落盘）与**持久**的「还该不该被驱动」（`meta.run`：`active` / `paused` / `closed`，派发前过 `dispatch_gate`）。

**不管**：不读文件（`std::fs`）、不发网络（ureq）、不碰 stdin/stdout——机制一律下沉各能力的 `detail/`；**不持任何别人的端口**（R12）；不替用户选人、选形态。

## 二、入站契约与状态归属

`api::SessionOps`（会话中心）、`api::ConductorOps`（协调用例）、`api::LogOps`（埋点门面）、`Ops`（组装后交给呈现层）、`ConductorHandle`（自持线程 + 命令队列 + 事件台）与队列代理。状态：会话在世表、命令队列、运行态——**只有它写**。
出站端口只有它自己的 `ports::ProxyHost`（核心代理工具的外部动作）；生产实现是 `service/proxy.rs` 的 `ProxyBridge`（**队列桥**：工具的每个动作回到核心线程执行，核心状态所有权不变），工具逻辑在 `domain/proxy.rs`（纯逻辑）与 `service/proxy.rs`（`ProxyTools` 校验 + 幂等账）。代理会话经通用成员循环挂 `ProxyHandler`（`kernel::ports::ToolHandler`），没有「代理专用」的循环分支。剩余缺口见 `testgaps.yaml`。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`collab`、`kernel`、`llm`、`prompt`、`registry`、`session`、`slate`、`taskchain`、`tools`、`workspace`
- 谁在用我：`cli`、`web`

## 四、改动本单元时必须同步

- 动 `Ops`（增删方法、事件、视图字段）→ 同时改 `cli`、`web`、`src/tests/api.rs`、`docs/presentation/contracts.md`（路由表由 `src/tests/routes.rs` 机器比对）；跨会话回档的编排与 `session` 的纯行算术要一起改（见 `session-model.md` 五）。
- 业务缺口账：`src/capabilities/conductor/testgaps.yaml`——业务 AI **只记缺口、不写测试**，由测试 AI 实现测试并销账；格式见 [docs/testing/gaps-acceptance.md](../../docs/testing/gaps-acceptance.md) §十二。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
