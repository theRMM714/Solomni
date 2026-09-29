# conductor（协调业务）

> 跨参与方的状态与编排：会话中心、生成驱动、`Ops` 的组装。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：会话中心（会话在世表、命令队列、运行态）、生成驱动（单 agent 与协作的长步骤都在工作线程跑，队列只占「取/交」两步）、跨能力用例与跨会话回档编排、把各能力的能力接口组装成呈现层唯一入口 `Ops`。

**不管**：不读文件（`std::fs`）、不发网络（ureq）、不碰 stdin/stdout——机制一律下沉各能力的 `detail/`；**不持任何别人的端口**（R12）；不替用户选人、选形态。

## 二、入站契约与状态归属

`api::SessionOps`（会话中心）、`api::ConductorOps`（协调用例）、`api::LogOps`（埋点门面）、`Ops`（组装后交给呈现层）、`ConductorHandle`（自持线程 + 命令队列 + 事件台）与队列代理。状态：会话在世表、命令队列、运行态——**只有它写**。

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
