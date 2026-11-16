# 单元地图 · turn

> 定位：turn（回合状态）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/turn/api.rs`

> 本文是 **turn** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**回合状态的唯一持有者**：会话在世表、运行态与创建占位、每会话的裁决队与 shell 活进程槽都只有这里增删，
> 别的业务经 `api` 取用。会话本体（`Turn`）、一席的装配材料、**回合推进与生成驱动**、协作链、事件台与逐轮落盘也在这里收拢：
> 准入与落盘、按需重建、派发闸、三形态重建、代理装配、单 agent / 节点 / 成员 / 协作 / 压缩五条生成链都归它；命令队列与动作分发在 `runtime`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/turn.rs` | 能力入口与公开面 |
| `src/capabilities/turn/api.rs` | 入站契约：`Session`（两种在世形态）、`Turn`（会话本体）、装配材料（`SessionDeps` / `MemberSpec` / `SingleSpec` / `member_tools` / `single_session`）、转录回放（`replay_dialogue`）与代理会话常量（`PROXY_PRIVATE_DIR` / `SPEAKER`）的重导出；事件台的读写两端（`EventLine` 与端口 `EventFeed` / `Events`、工厂 `event_bus`）、回执与入参值类型（`Output` / `Advance` / `Prepared` / `CollabWork` / `AskReq` / `RestoreRequest`）、生成入口与点火器、宿主端口 `TurnHost` 与提问端口工厂 `session_ask` |
| `src/capabilities/turn/service.rs` | 服务分区声明 |
| `src/capabilities/turn/service/bus.rs` | 事件发布台本体：入箱与裁剪（上限一万条、保留五千条）、回档重置水印、按序号取批与历史合流的实时尾巴；结构不出单元，只经 `EventFeed`（读）与 `Events`（写）两端 |
| `src/capabilities/turn/service/state.rs` | 状态本体与访问器：在世表、运行态、创建占位、裁决队表、每会话 shell 槽 |
| `src/capabilities/turn/service/session.rs` | 会话本体 `Turn`：持状态、装配要的能力面（`workspace` / `vm` / `residents` / `secrets` / `tools` / `systools` / `prompt` / `llm` / `log` / `compact` / `base_roots`）与重建要的四处事实（`root` / `history` / `modules` / `packages`），转发全部状态访问器，给 `running_refusal` 的唯一实现 |
| `src/capabilities/turn/service/admit.rs` | 会话准入：`admit`（落盘 + 进在世表）、`ensure_session`（不在世就重建再放入）、`dispatch_gate`（终态会话拒绝派发）、`resume_subtree`（把停掉的子树拉回运行态） |
| `src/capabilities/turn/service/facts.rs` | 一席事实的取用：沙箱清单、权限面、外部工作区授权粒度、工具面（`member_tools`）与上下文预算；只经各能力 `api` 取，不持端口 |
| `src/capabilities/turn/service/rebuild.rs` | 从落盘重建会话：外部工作区绑定复检（`workspace_binding_of`）、三形态重建（`rebuild_session`）、转录回放（`replay_reads` / `replay_dialogue`）与代理会话装配（`build_proxy`） |
| `src/capabilities/turn/service/assemble.rs` | 一席的装配：`SessionDeps`（能力面）、`MemberSpec` / `SingleSpec`（取来的本席事实）、`member_tools`（工具面）、`single_session`（单 agent 会话对象）与 `Turn::build_single`（把一席事实装配成会话；`work` 的建工作两段经它） |
| `src/capabilities/turn/service/turn.rs` | 单 agent 的回合收发：工具形态刷新、生成前准备、节点派发准备、取放会话的两段（`take_single` / `take_collab`）、裁决作废、派发目标、事件收编与落盘手柄、改需求的重建请求 |
| `src/capabilities/turn/service/flow.rs` | 协作流水线推进与协作动作：链推进、节点派发与子会话、协作步（`take_*` / `put_*`）、裁决卡动作与改需求 |
| `src/capabilities/turn/service/history.rs` | 事件收编与增量落盘手柄（落盘策略、手柄与会话树派生归 `session`）；运行态改写 `set_subtree_run` 与派发目标 |
| `src/capabilities/turn/service/ask_user.rs` | 工具层提问端口的会话侧实现：推一条问题并阻塞等一个选项 id |
| `src/capabilities/turn/service/model.rs` | 一次模型调用的通道参数与思考片段：流式开关、超时、按登记处取思考等级（建会话与回合推进共用这一份判据） |
| `src/capabilities/turn/service/generation.rs` | 生成的点火器：节点执行、继续、代理回合、协作推进、裁决处置各起一条脱离调用方的线程 |
| `src/capabilities/turn/service/generation/single.rs` | 单 agent 与节点的生成循环：核心只占「取 / 交」两步，生成在工作线程上跑，逐轮外送并落盘 |
| `src/capabilities/turn/service/generation/member.rs` | 成员回合：为协作泵的一个请求取该 agent 的会话、跑完把回合结果带回（子会话自己的事件台与逐轮落盘） |
| `src/capabilities/turn/service/generation/compact.rs` | 手动压缩：让模型自己压，队列只占「取 / 交」，压不动如实说 |
| `src/capabilities/turn/service/generation/collab.rs` | 协作泵：开始讨论 / 回答 / 继续，与主线程握手（`AskReq` / `MemberTurn`），事件边产边送 |
| `src/capabilities/turn/domain.rs` | 纯派生分区声明 |
| `src/capabilities/turn/domain/assemble.rs` | 装配的纯函数：`face_tools`（发放名单过滤）、`service_ops_of`（就绪服务 → 工具）、`serviced_tools`（服务工具与提示行注入模块表） |
