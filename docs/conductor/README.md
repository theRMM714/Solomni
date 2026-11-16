# conductor（协调业务）

> 定位：应用编排——命令队列、协作链推进、代理工具的宿主（队列桥）、动作分发、历史生命周期的时机，以及把各能力组装成 `Ops`。
> 效力：规范｜核对：2026-11-16｜依据：`src/capabilities/conductor/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话中心——命令队列与派发闸；会话在世表、运行态与创建占位在 `turn` 里，只经它的 `api` 读写。
- 管：**协作链推进与协作动作**（`src/capabilities/conductor/service/flow.rs`）：链推进、节点派发与子会话、协作步与裁决卡动作。
- 管：**代理工具的宿主与时序**（`src/capabilities/conductor/service/proxy.rs`）：队列桥 `ProxyBridge` 实现 `proxy` 的 `ProxyHost`（清单 / 建会话 / 转达 / 观察 / 控制 / 消息都回到核心线程），以及代理工具面在代理会话里的挂载（`src/capabilities/conductor/api/handle.rs` 的 `inject_proxy_handler`）；工具面、授权判据与幂等账已下沉到 `proxy`（第 9 层，见 [../proxy/README.md](../proxy/README.md)），代理会话的装配在 `turn`（`build_proxy`），「建一个代理工作」的用例在 `work`。
- 管：生成驱动——**长步骤一律 own-and-return**（见「一、长步骤的边界」）：生成、协作泵、压缩、起 / 停 guest 都不在队列里跑，队列只占「取 / 交」两步；建会话 / 启用历史会话 / 改配 / 删会话的 guest 启停在调用方线程上（起停对象在 `src/capabilities/work/service/guest_job.rs`）。
- 管：动作分发（`src/capabilities/conductor/api/action.rs`）与事件收编 / 增量落盘（`src/capabilities/conductor/service/history.rs`）。
- 管：**历史生命周期的时机**——回档三模式、历史列表 / 只读打开与删除的调用点在这里（用例与顺序在 `lifecycle`，第 9 层，见 [../lifecycle/README.md](../lifecycle/README.md)）。
- 管：**同名薄转发**——工作用例（建工作两段、改配、视图、运行报告、建议、激活）的实现在 `work`，这里保留同名方法让队列、呈现层与测试的调用点不变。
- 管：把各能力的能力接口组装成呈现层唯一入口 `Ops`。
- 运行态分两层：**短暂**的「这一刻谁在干活」（`SessionEvent::Working`，不落盘）与**持久**的「还该不该被驱动」（`meta.run`：`active` / `stopped` / `closed`，派发前过 `dispatch_gate`）。
- 不管：不读文件（`std::fs`）、不发网络（ureq）、不碰 stdin/stdout——机制一律下沉各能力的 `detail/`（见 `src/capabilities/conductor/service.rs`）；不持任何别人的端口（R12）；不替用户选人、选形态。
- 不管：**工作是什么样**——工作形态的判据、建工作两段、改配、视图与运行报告在 `work`（第 9 层，见 [../work/README.md](../work/README.md)）。

## 入站契约与状态归属

- 入站面：`api::SessionOps`（会话中心）、`api::ConductorOps`（协调用例）、`api::LogOps`（埋点门面）、`Ops`（组装后交给呈现层）、`ConductorHandle`（自持线程 + 命令队列 + 事件台）与队列代理。
- 状态归属：命令队列只有它写；会话在世表、运行态、创建占位、裁决队与 shell 槽归 `turn`，它只经 `turn` 的 `api` 读写。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`collab`、`env`、`kernel`、`lifecycle`、`llm`、`modules`、`permission`、`prompt`、`proxy`、`provision`、`registry`、`residents`、`secrets`、`session`、`slate`、`taskchain`、`tools`、`turn`、`vm`、`work`、`workspace`
- 谁在用我：`cli`、`web`

## 一、长步骤的边界

- 核心线程**只做毫秒级的状态变换**；凡是等外部世界的步骤都要 own-and-return（取走对象 → 在工作线程或调用方线程上跑 → 结果作为事件回来）。
- 生成 / 协作泵 / 压缩 / 节点：句柄自己的工作线程（`solomni-gen` / `solomni-collab` / `solomni-compact` / `solomni-node` …）。
- 起 / 停 guest：分两段——`prepare_*`（核心：校验、占名、出计划、建工作区）与 `commit_*`（核心：落盘、装配会话、开场事实），中间那段（`src/capabilities/work/service/guest_job.rs` 的 `GuestBoot` / `GuestStop`）在调用方线程上跑；起一台虚拟机会话时核心照常应答，别的会话、状态查询、配置读取都不排队，失败则在提交前撤销（占名与空工作区目录都不留）。
- 启用历史会话：`activate_session` 同样两段——核心读元信息 / 判 closed / 准备 guest，调用方线程起 guest，核心提交（重建会话、解冻整棵子树）；启用本身不跑模型，失败不留「已解冻 / 已装进内存」的半成品。
- 删除会话：核心做完撤授权 / 回收常驻服务 / 移出内存并收集待删目录（毫秒级）；guest 销毁与目录删除交给调用方线程按序执行——先停 guest，再删目录（Windows 上 guest 可能还占着目录里的文件）。

## 改动本单元时必须同步

- 动 `Ops`（增删方法、事件、视图字段）必须同时改 `cli`、`web`、`src/tests/api/`、`docs/presentation/contracts.md`（路由表由 `src/tests/routes.rs` 机器比对）；回档的时机与 `lifecycle` 的用例、`session` 的纯行算术要一起改（见 [docs/lifecycle/README.md](../lifecycle/README.md) 与 [docs/session/session-model.md](../session/session-model.md) 的「回档：三种模式 + 工作级标记」）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
