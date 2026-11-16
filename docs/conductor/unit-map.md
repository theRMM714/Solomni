# 单元地图 · conductor

> 定位：conductor（协调业务）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/conductor/api.rs`

> 本文是 **conductor** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：命令队列与跨会话编排；会话在世表与运行态归 `turn`。
> 「工作是什么样」的用例（建工作两段、改配、视图、运行报告、guest 生命周期、建议与激活）已下沉到 `work`（第 9 层），本单元只留应用编排与同名薄转发（见 [../work/unit-map.md](../work/unit-map.md)）。
> 「历史生命周期」的用例（回档三模式、历史列表与只读打开、删除的报账与收尾）已下沉到 `lifecycle`（第 9 层，见 [../lifecycle/unit-map.md](../lifecycle/unit-map.md)），本单元只给时机。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conductor.rs` | 能力入口与公开面 |
| `src/capabilities/conductor/api.rs` | 入站契约：`SessionOps` / `ConductorOps` / `ActionOps` / `LogOps`、`Ops` 组装面与事件台（`EventBus`） |
| `src/capabilities/conductor/api/action.rs` | 动作分发：参数校验 → 按 `callers` 授权 → 执行 → 审计（各只有一处） |
| `src/capabilities/conductor/api/handle.rs` | 核心手柄：装配命令队列（`kernel` 的 `Actor`，线程与 panic 隔离在它里面）+ 事件台，线程边界上的短命令（补编、回档收尾、guest 启动、启用会话）；`stop` 不进队列 |
| `src/capabilities/conductor/api/generation.rs` | 生成的点火器：节点执行、继续、代理回合、协作推进、裁决处置各起一条脱离调用方的线程 |
| `src/capabilities/conductor/api/generation/single.rs` | 单 agent 与节点的生成循环：核心只占「取 / 交」两步，生成在工作线程上跑，逐轮外送并落盘 |
| `src/capabilities/conductor/api/generation/member.rs` | 成员回合：为协作泵的一个请求取该 agent 的会话、跑完把回合结果带回（子会话自己的事件台与逐轮落盘） |
| `src/capabilities/conductor/api/generation/compact.rs` | 手动压缩：让模型自己压，队列只占「取 / 交」，压不动如实说 |
| `src/capabilities/conductor/api/generation/collab.rs` | 协作泵：开始讨论 / 回答 / 继续，与主线程握手（`AskReq` / `MemberTurn`），事件边产边送 |
| `src/capabilities/conductor/api/contract_impls.rs` | 各能力入站契约的队列代理：每个方法克隆入参 → 进命令队列 → 取回结果（呈现层因此只持 `Ops`，拿不到 `Conductor`） |
| `src/capabilities/conductor/service.rs` | 会话中心：命令队列、取放会话的两段（`take_single` / `take_collab`）、裁决作废与注入依赖；会话本体（`Turn`）、在世表 / 运行态 / 裁决队 / shell 槽、派发闸与准入都在 `turn` 里，工作用例的实现在 `work` 里，本文件只有薄转发 |
| `src/capabilities/conductor/service/action.rs` | 动作的核心实现：把规整后的建会话载荷变成一条会话 |
| `src/capabilities/conductor/service/turn.rs` | 单 agent 的回合收发：工具形态刷新、生成前准备、节点派发准备、同步生成入口、继续 |
| `src/capabilities/conductor/service/compact.rs` | 压缩通道工厂：全局设置指向的压缩模型解析与更新 |
| `src/capabilities/conductor/service/flow.rs` | 协作流水线推进与协作动作：链推进、节点派发与子会话、协作步（`take_*` / `put_*`）、裁决卡动作与改需求 |
| `src/capabilities/conductor/service/env.rs` | 薄转发：guest 诊断准备与沙箱 / 权限 / 预算三个薄入口（事实在 `turn::service::facts`）；单会话装配（`Turn::build_single`）已归 `turn` |
| `src/capabilities/conductor/service/history.rs` | 事件收编与增量落盘手柄（落盘策略、手柄与会话树派生归 `session`）；运行态改写 `set_subtree_run` 与派发目标也在这一处 |
| `src/capabilities/conductor/service/ask_user.rs` | 工具层提问端口的会话侧实现：推一条问题并阻塞等一个选项 id |
| `src/capabilities/conductor/ports.rs` | 出站端口：核心代理工具要做的外部动作（`ProxyHost`） |
| `src/capabilities/conductor/domain/action.rs` | 动作回包的纯逻辑：探测结论 → 响应 JSON |
| `src/capabilities/conductor/domain/proxy.rs` | 核心代理工具的纯逻辑：入参解析、授权校验、载荷规整与回执拼装 |
| `src/capabilities/conductor/service/proxy.rs` | 代理工具执行面与队列桥 `ProxyBridge`（把动作交给核心线程）；代理会话的装配在 `turn::service::rebuild`（`build_proxy`） |
