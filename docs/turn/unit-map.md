# 单元地图 · turn

> 定位：turn（回合驱动）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/turn/turn/api.rs`

> 本文是 **turn** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**回合驱动的唯一实现**：会话准入与派发闸、按需重建、一席事实的取用、回合与协作的推进规则、
> 五条生成链与七条点火器、事件台与逐轮落盘手柄。会话在世表与运行态、创建占位、裁决队与流程契约都在
> `turn_state`；会话本体 `AgentSession` 与一个身份装配在 `agentloop`；转录落盘归 `conversation::session`；
> 命令队列、动作分发与线程池归宿主 `runtime`。一席的运行环境规则在 `tools`（`src/units/business/tools/domain/env.rs`），工作台声明与沙箱清单在 `workspace`；本单元只取事实后转调。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/turn.rs` | 单元分区声明：`turn`（回合驱动）一个子业务 |
| `src/units/business/turn/turn.rs` | 子业务入口与公开面 |
| `src/units/business/turn/turn/api.rs` | 入站契约：宿主端口 `TurnHost`（核心线程改状态、事件台、guest、裁决队与提问端口、生成登记、点火、回答裁决卡）与命令面 `TurnOps`（`say` / `continue_flow` / `set_task` / `answer_card` / `open_queue` / `withdraw_agree` / `compact` / `is_running`）；事件台读写两端（`EventLine` 与 `EventFeed` / `Events`、`event_bus`）、生成入口与七条点火器、`replay_dialogue`、代理会话常量 `PROXY_PRIVATE_DIR` / `SPEAKER`，以及回执与入参值类型 `Output` / `Advance` / `Prepared` / `CollabWork` / `AskReq` / `RestoreRequest` |
| `src/units/business/turn/turn/service.rs` | 推进与生成驱动的分区声明（九个模块） |
| `src/units/business/turn/turn/service/bus.rs` | 事件发布台本体：入箱与裁剪（上限一万条、保留五千条）、回档重置水印、按序号取批与历史合流的实时尾巴；结构不出单元，只经 `EventFeed`（读）与 `Events`（写）两端 |
| `src/units/business/turn/turn/service/admit.rs` | 会话准入：`admit`（落盘 + 进在世表）、`ensure_session`（不在世就重建再放入）、`dispatch_gate`（终态会话拒绝派发）、`resume_subtree`（把停掉的子树拉回运行态） |
| `src/units/business/turn/turn/service/facts.rs` | 一席事实的取用：`sandboxes`（把每席计划交 `workspace::api::seat_benches`）/ `permissions_for`（`session_model::api::permissions`）/ `shared_scope_for`（`workspace::api::scope_from`）/ `workspace_decl_chain` / `access_for_agent` / `member_env`（`tools::api::member_env`）/ `member_face` 的取数与转调、`context_budget` 的窗口与阈值，以及 `build_single` 取完事实后交 `agentloop` 装配；只经各单元 `api` 取，不持端口 |
| `src/units/business/turn/turn/service/rebuild.rs` | 从落盘重建会话：`workspace_binding_of`（外部根复检）、`rebuild_session`（单 agent / 协作 / 代理三形态）、`replay_dialogue`（转录还原与压缩边界）、`build_proxy`（代理会话装配）与 `PROXY_PRIVATE_DIR` / `SPEAKER` |
| `src/units/business/turn/turn/service/turn.rs` | 单 agent 回合的收发：`refresh_tool_mode`（形态变了先提示）、`prepare_single` / `prepare_node`（生成前准备与节点派发准备）、`single_say` / `continue_flow`、放取会话（`take_single` / `take_flow`）、交回时按落盘运行态作废裁决队（`void_gates_of` / `void_gates_in_subtree`）、`compact_plan` 与 `note_system` |
| `src/units/business/turn/turn/service/flow.rs` | 协作流水线推进与协作动作：链推进 `advance_chain`、节点派发 `drive_node` / `spawn_ready_nodes` / `spawn_sub_session`、协作步 `collab_advance` / `collab_resume`（取用 `take_flow_raw`）、裁决卡动作 `collab_set_task` / `collab_answer` / `take_card` / `dispose_and_advance` / `withdraw_agree` / `collab_pending`，以及放回 `put_single` / `put_single_recorded` / `put_collab` |
| `src/units/business/turn/turn/service/history.rs` | 事件收编与逐轮落盘手柄：`record_events`（收编 + 定稿事件落盘）、`persister`（取增量落盘手柄）、`set_subtree_run`（停止 / 继续时整棵子树的运行态）、`dispatch_target` 与 `notice_row`；落盘实现在 `conversation::session` |
| `src/units/business/turn/turn/service/model.rs` | 一次模型调用的通道参数与思考片段：`llm_opts` / `llm_opts_for` / `thinking_fragment_for`（流式开关、预算、按登记处解析思考档）；建会话与回合推进共用这一份判据 |
| `src/units/business/turn/turn/service/generation.rs` | 生成的点火器：节点执行、继续、代理回合、协作推进、协作回答处置、裁决处置各起一条脱离调用方的线程；线程名与错误文案是对外可见的运行时事实 |
| `src/units/business/turn/turn/service/generation/single.rs` | 单 agent 与节点的生成循环：核心只占「取 / 交」两步，生成在工作线程上跑，逐轮外送并落盘 |
| `src/units/business/turn/turn/service/generation/member.rs` | 成员回合：为协作泵的一个请求取该 agent 的会话、跑完把回合结果带回（子会话自己的事件台与逐轮落盘） |
| `src/units/business/turn/turn/service/generation/compact.rs` | 手动压缩：让模型自己压，队列只占「取 / 交」，压不动如实说 |
| `src/units/business/turn/turn/service/generation/collab.rs` | 协作泵：开始讨论 / 回答 / 继续，与主线程握手（`AskReq` / `MemberTurn`），事件边产边送 |
