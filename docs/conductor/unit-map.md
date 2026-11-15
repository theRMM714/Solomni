# 单元地图 · conductor

> 定位：conductor（协调业务）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/conductor/api.rs`

> 本文是 **conductor** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：命令队列、生成驱动与跨会话编排；工作形态（形态决定、建工作、改配、视图与运行报告）在 `work`，会话本体（在世表、运行态、占名表、裁决队、装配与重建、树序）在 `turn`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conductor.rs` | 能力入口与公开面 |
| `src/capabilities/conductor/api.rs` | 入站契约：`WorkOps` / `TurnOps` / `LifecycleOps` / `ActionOps` / `LogOps`、`Ops` 组装面与事件台（`EventBus`） |
| `src/capabilities/conductor/api/action.rs` | 动作分发入口：`ActionOps` 的 `catalog` / `act`、按 `callers` 授权、`execute_action` 按动作归属三分派与审计；执行体在 `api/action/` 三份 |
| `src/capabilities/conductor/api/action/session.rs` | 会话 / 工作动作的执行面：建会话、发消息、裁决、回档、改配置、上传与模块启停（参数搬运 + 入队） |
| `src/capabilities/conductor/api/action/module.rs` | 模块工具与常驻服务操作的执行面：`execute_module_action` 分派、取清单、授权、围栏执行与回包 |
| `src/capabilities/conductor/api/action/registry.rs` | 登记处动作适配：调 registry 的纯解析（`apply_settings` / `model_draft` / `probe_view`）后入队 |
| `src/capabilities/conductor/api/handle.rs` | 核心手柄：自持线程 + 命令队列 + 事件台，线程边界上的短命令（补编、回档收尾、guest 启动、启用会话）；`stop` 不进队列 |
| `src/capabilities/conductor/api/generation.rs` | 生成的点火器：节点执行、继续、代理回合、协作推进、裁决处置各起一条脱离调用方的线程 |
| `src/capabilities/conductor/api/generation/single.rs` | 单 agent 与节点的生成循环：核心只占「取 / 交」两步，生成在工作线程上跑，逐轮外送并落盘 |
| `src/capabilities/conductor/api/generation/member.rs` | 成员回合：为协作泵的一个请求取该 agent 的会话、跑完把回合结果带回（子会话自己的事件台与逐轮落盘） |
| `src/capabilities/conductor/api/generation/compact.rs` | 手动压缩：让模型自己压，队列只占「取 / 交」，压不动如实说 |
| `src/capabilities/conductor/api/generation/collab.rs` | 协作泵：开始讨论 / 回答 / 继续，与主线程握手（`AskReq` / `MemberTurn`），事件边产边送 |
| `src/capabilities/conductor/api/contract_impls.rs` | 各能力入站契约的队列代理：每个方法克隆入参 → 进命令队列 → 取回结果（呈现层因此只持 `Ops`，拿不到 `Conductor`） |
| `src/capabilities/conductor/service.rs` | 会话中心：命令队列、跨会话编排与会话装配的入口；`Conductor` 只持一个 `turn::api::Turn` 与一个 `work::api::Work`，五张表、取放协议、装配与重建、树序都在 turn（`src/capabilities/turn/service/session.rs`），工作用例在 work（`src/capabilities/work/service.rs`） |
| `src/capabilities/conductor/service/turn.rs` | 单 agent 的回合收发：工具形态刷新、生成前准备、节点派发准备、同步生成入口、继续；取放与重建走 turn |
| `src/capabilities/conductor/service/compact.rs` | 压缩通道工厂：全局设置指向的压缩模型解析与更新 |
| `src/capabilities/conductor/service/flow.rs` | 协作流水线推进与协作动作：链推进、节点派发、协作步编排、裁决卡动作与改需求；取放、子会话派生与树序走 turn |
| `src/capabilities/conductor/service/rewind.rs` | 跨会话回档编排：按同一回合 id 收窗整棵子树、折叠子会话、告知副作用；按转录重建归 turn（`src/capabilities/turn/service/rebuild.rs`） |
| `src/capabilities/conductor/service/history.rs` | 历史列出 / 打开 / 删除的转发与装配（收尾类型来自 `lifecycle`）、事件收编与增量落盘手柄；树序、绑定复检与裁决队句柄在 turn |
| `src/capabilities/conductor/service/ask_user.rs` | 工具层提问端口的会话侧实现：推一条问题并阻塞等一个选项 id |
| `src/capabilities/conductor/service/proxy.rs` | 核心侧代理适配：`proxy_*` 用例与 `ProxyBridge`（把 `proxy` 的端口调用丢回核心线程），工具执行面在 `proxy` 能力；代理会话装配在 turn（`src/capabilities/turn/service/proxy.rs`） |
