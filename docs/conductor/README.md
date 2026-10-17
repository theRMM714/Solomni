# conductor（协调业务）

> 跨参与方的状态与编排：会话中心、生成驱动、`Ops` 的组装。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：会话中心（会话在世表、命令队列、运行态）、**工作形态**（single / collab / **proxy**：proxy 没有名单，用户选它就是授予全权，见 §二）、生成驱动（**长步骤一律 own-and-return**：生成、协作泵、压缩、起/停 guest 都不在队列里跑，队列只占「取/交」两步——建会话/改配/删会话的 guest 启停在调用方线程上，见 §二 的「长步骤的边界」）、跨能力用例与跨会话回档编排、把各能力的能力接口组装成呈现层唯一入口 `Ops`。运行态分两层：**短暂**的「这一刻谁在干活」（`SessionEvent::Working`，不落盘）与**持久**的「还该不该被驱动」（`meta.run`：`active` / `paused` / `closed`，派发前过 `dispatch_gate`）。

**不管**：不读文件（`std::fs`）、不发网络（ureq）、不碰 stdin/stdout——机制一律下沉各能力的 `detail/`；**不持任何别人的端口**（R12）；不替用户选人、选形态。

## 二、入站契约与状态归属

`api::SessionOps`（会话中心）、`api::ConductorOps`（协调用例）、`api::LogOps`（埋点门面）、`Ops`（组装后交给呈现层）、`ConductorHandle`（自持线程 + 命令队列 + 事件台）与队列代理。状态：会话在世表、命令队列、运行态——**只有它写**。

### 长步骤的边界

核心线程**只做毫秒级的状态变换**；凡是等外部世界的步骤都要 own-and-return（取走对象 → 在工作线程或调用方线程上跑 → 结果作为事件回来）。

- 生成 / 协作泵 / 压缩 / 节点：句柄自己的工作线程（`solomni-gen` / `solomni-collab` / `solomni-compact` / `solomni-node` …）。
- **起 / 停 guest**：分两段——`prepare_*`（核心：校验、占名、出计划、建工作区）与 `commit_*`（核心：落盘、装配会话、开场事实），
  中间那段（`service/guest_job.rs` 的 `GuestBoot` / `GuestStop`）在**调用方线程**上跑。所以起一台虚拟机会话时核心照常应答，
  别的会话、状态查询、配置读取都不排队；失败则在提交前撤销（占名与空工作区目录都不留）。
- 删除会话：核心做完撤授权 / 回收常驻服务 / 清目录（毫秒级），guest 销毁同样交给调用方线程（要等它落盘，秒级）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`collab`、`kernel`、`llm`、`modules`、`permission`、`prompt`、`provision`、`registry`、`residents`、`secrets`、`session`、`slate`、`taskchain`、`tools`、`vm`、`workspace`
- 谁在用我：`cli`、`web`

## 四、改动本单元时必须同步

- 动 `Ops`（增删方法、事件、视图字段）→ 同时改 `cli`、`web`、`src/tests/api.rs`、`docs/presentation/contracts.md`（路由表由 `src/tests/routes.rs` 机器比对）；跨会话回档的编排与 `session` 的纯行算术要一起改（见 `session-model.md` 五）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |