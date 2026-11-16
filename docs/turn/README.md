# turn（回合状态）

> 定位：回合状态的唯一持有者——会话在世表、运行态与创建占位、每会话的裁决队与 shell 活进程槽；会话本体、准入与重建、一席的装配材料，以及回合推进、生成驱动、协作链、事件台与逐轮落盘也在这里收拢。
> 效力：规范｜核对：2026-11-16｜依据：`src/capabilities/turn/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**会话在世表**——键是会话名，值是一份在世会话的两种形态（单 agent / 协作）；生成期间对象被取出，表里不留影子。
- 管：**运行态与创建占位**——正在生成的会话集合、正在创建 / 启动的会话集合；改配、回档、删除据此拒绝。
- 管：**裁决队**——全会话一条裁决通道表（`session` 的 `Decisions`），同会话一次只问一条。
- 管：**每会话的 shell 活进程槽**——同一会话的多次 shell 调用共用一个活进程（`session::api::ShellSlot`）。
- 管：**会话本体** `Turn`——状态 + 装配要的能力面（`workspace` / `vm` / `residents` / `secrets` / `tools` / `systools` / `prompt` / `llm` / `log` / `compact` / `base_roots`）+ 重建要的四处事实（`root` / `history` / `modules` / `packages`），以及 `TurnState` 全部访问器的一行转发与 `running_refusal` 的唯一实现。
- 管：**准入与派发闸**——`admit` 先落盘再进在世表，`ensure_session` 不在世就按转录重建，`dispatch_gate` 按运行态拒绝派发，`resume_subtree` 把停掉的子树拉回运行态。
- 管：**按需重建**——`rebuild_session` 按落盘的事件与元数据重建单 agent / 协作 / 代理三种形态，`workspace_binding_of` 做加载复检，`replay_dialogue` / `replay_reads` 还原对话与读事实，`build_proxy` 装配代理会话。
- 管：**一席事实的取用**——沙箱清单、权限面、外部工作区授权粒度、`env::api::member` 算出的工具环境（`MemberEnvParts`）与上下文预算（`src/capabilities/turn/service/facts.rs`）；事实都经各能力的 `api` 取。
- 不管：**一席的装配**——工具面与三种在世形态的构造归 `session`（第 4 层：`src/capabilities/session/service/assemble.rs` 的 `single_session` / `rebuild_single` / `proxy_session`）；本单元只取事实、组 `MemberSpec` 后转调。`Turn::build_single` 仍是 `work` 建工作两段的入口。
- 不管：会话对象（`session` 的 `AgentSession`、`collab` 的 `CollabSession`）怎么长、怎么跑——本单元只放取、标记与重建。
- 管：**回合推进**——单 agent 与节点的生成准备（工具形态刷新、预算刷新、身份、前缀事件、`Prepared`）与同步生成入口、取放会话的两段（`take_single` / `take_collab` / `put_single` / `put_collab`）、裁决作废与派发目标、协作流水线推进与协作动作（链推进、节点派发、裁决卡动作、改需求）。
- 管：**生成驱动**——单 agent / 节点、成员、协作泵、压缩五条生成链，以及七条点火器（节点执行、继续、代理回合、协作推进、裁决处置）；线程名 `solomni-*` 是行为契约的一部分，线程本身经宿主的 `TurnHost::spawn` 起。
- 管：**事件台** `EventBus`——核心独占生产、按会话与序号增量取，回档后按会话重放；`seq` 单调，消费方只看增量。
- 管：**逐轮落盘**——`Persister` / `persist_events` 手柄、事件收编（`record_events` / `notice_row`）与游标；落下的行 id、标记顺序与事件顺序是行为契约。
- 不管：命令队列与时序编排——谁在什么时候调生成入口、命令怎么排队，由调用方（`runtime`）决定；本单元只在自己起的线程里推完一条链。
- 不管：登记处本身（`registry`）——工具模式、上下文预算与核心默认模型经调用方传进来的 `&dyn Registry` 现取，`Turn` 不持有它。

## 入站契约与状态归属

- 入站面：`api::TurnState`（`session` / `session_mut` / `holds` / `put` / `take` / `is_running` / `mark_running` / `unmark_running` / `running_iter` / `sessions_iter` / `is_starting` / `mark_starting` / `unmark_starting` / `desk` / `desk_for` / `shell_slot` / `shell_view`）、`api::Session`，以及 `api::Turn`（同样的访问器 + `set_base_roots` / `session_ports` / `running_refusal` / `admit` / `ensure_session` / `dispatch_gate` / `resume_subtree` / `rebuild_session` / `workspace_binding_of` / `build_proxy` / `sandboxes` / `permissions_for` / `workspace_grant` / `member_env` / `build_single` / `context_budget`）；别的业务与测试只经 `api` 取用（R1）。
- 入站面（回合推进）：`api::Turn` 的 `take_single` / `take_collab` / `put_single` / `put_collab` / `void_gates_of` / `void_gates_in_subtree` / `desk_of` / `note_system` / `bump_turn_of_child` / `compact_plan` / `refresh_tool_mode` / `prepare_single` / `prepare_node` / `open_queue` / `take_card` / `dispose_and_advance` / `update_task_outcome` / `cancelled_card` / `advance_chain` / `spawn_ready_nodes` / `notify_proxy_of_child` / `set_subtree_run` / `dispatch_target` / `record_events` / `notice_row` / `persister`。
- 入站面（生成与事件台）：`api::single_generation` / `api::node_generation` / `api::collab_generation` / `api::compact`，七条点火器 `api::spawn_detached_*` / `api::start_child_collab`，`api::EventBus`（`push` / `push_rewound` / `head` / `snapshot` / `tail_excluding`）、`api::EventLine`，以及值类型 `Output` / `Advance` / `Prepared` / `CollabWork` / `AskReq` / `RestoreRequest`。
- 入站面（宿主端口）：`api::TurnHost`——业务循环自己做不了、必须宿主在核心线程上代做的入口（取放状态、事件台、guest 起停、裁决队与提问端口、生成登记、线程点火、回答裁决卡）；`api::session_ask` 是提问端口的工厂。
- 入站面（重建与代理）：`api::replay_dialogue(&[Value], ToolMode) -> (对话, 标记, 回复号, 行号, 压缩截至行, 压缩截至回复)`、`api::PROXY_PRIVATE_DIR` / `api::SPEAKER`（代理会话的私有目录名与说话人，只有一处）。
- 入站面（装配）：本单元只出 `Turn::member_env -> MemberEnvParts`（`env::api::member` 的取数结果，逐字段拷成值）与 `Turn::build_single`；装配本身在 `session::api`（`single_session` / `rebuild_single` / `proxy_session`，收 `SessionPorts` 与该能力自己的 `MemberSpec`）；`runtime` 只负责取事实与转调。
- 状态归属：**它自己写，别人只经 `api` 取**（R4）——在世表、运行态、创建占位、裁决队与 shell 槽只有这里增删；能力面是构造时注入的借用方，`Turn` 不拥有它们的内部状态。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`session`（`AgentSession` / `ShellSlot` / `ShellLive` / `Decisions` / `decisions` / `GateTicket` / `Gates` / `ModuleTools` / `ServiceOp` / `ToolFace` / `SessionParams` / `SessionEvent` / `SessionMeta` / `AgentMeta` / `RunState` / `History` / `CompactChatFactory` / `ToolCallView` / `subtree_of` / `truncate_events` / `issued_in` / `role_of` / `session_kind_of` / `guest_lease_for` / `work_root` / `work_root_of` / `max_reply` / `reply_msgs` / `tool_result_body` / `last_usage` / `last_shell` / `last_compaction` / `service_ops_for_modules` / `tool_table`）、`collab`（`CollabSession`）、`kernel`（`SessionId` / `Tier` / `Log` / `HostProbe`）、`workspace`（`Workspace` / `Sandbox` / `ReadFact`）、`tools`（`Tools` / `ToolExec` / `ToolBook` / `tool_notes`）、`prompt`（`Prompt` / `Segment`）、`permission`（`PermissionSet`）、`env`（`member` / `MemberFacts` / `sandboxes` / `permissions` / `workspace_grant` / `network_allowed` / `host_read_roots`）、`modules`（`Module` / `Modules` / `Roster`）、`packages`（`PackagePlan` / `Packages`）、`registry`（`Registry`：模型通道、工具模式、核心默认模型、上下文窗口、应用设置与快照）、`residents`（`ResidentOps` / `ServiceView` / `ServiceState`）、`secrets`（`SecretOps`）、`vm`（`Vm` / `GuestOs` / `path_codec_of` / `guest_os_of` / `unavailable_modules` / `host_plan_for`）、`llm`（`Llm` / `Channel` / `Msg` / `ToolMode`）；登记处本体不归本单元，`Turn` 只经传入的 `&dyn Registry` 现取
- 谁在用我：`runtime`（服务与命令门面都经 `self.turn.*`，装配经 `turn::api::Turn::build_single`（最终由 `session::api` 装配），生成与点火器经 `turn::api` 的入口，`Handle` 实现 `TurnHost`）、`presentation/web`（事件台增量取）、`src/tests/turn.rs` 与 `src/tests/{api,runtime,collab}` 里的事件与回合断言

## 一、取出与放回

- 生成开始时取走会话对象（`take`）并标运行态：核心表里只留「它在跑」这个事实，长步骤因此不占队列。
- 生成在工作线程上推进，事件经 `EventBus` 外送、逐轮经 `Persister` 落盘；结束时放回（`put`）并撤运行态。**取放的时机仍由调用方决定**，生成期间的取 / 交经 `TurnHost::core` 回核心线程，好让别的命令照常排队。
- 会话进表前先落盘（`admit`）：名字即目录，落盘失败就不进表；再开时 `ensure_session` 从落盘重建，不保留进程内影子。
- 运行态与创建占位是两套集合：前者挡改配 / 回档 / 删除，后者挡同名并发创建。

## 改动本单元时必须同步

- `src/capabilities/runtime/service/core.rs` → `turn` 字段是唯一读写口（访问器在 `Turn` 上，runtime 只转调）：增删访问器必须同改调用点与 [`unit-map.md`](unit-map.md)。
- `src/capabilities/turn/service.rs` / `session.rs` → `Turn::new` 的能力面与 `Core::new` 的注入列表一一对应：加一处必须同改另一处，且 `session_ports` 要一并给出。
- `src/capabilities/session/service/assemble.rs` → 装配顺序、条件与错误文案是行为契约（`build_single` 的形参变化必须同改 `src/capabilities/work/service/create.rs`）：改判据必须同改 `src/tests/tools/shell.rs`、`src/tests/runtime.rs` 的断言；本单元只负责取事实与转调（`src/capabilities/turn/service/facts.rs`）。
- `src/capabilities/turn/service/facts.rs` → 沙箱 / 权限 / 工作区授权 / 工具环境（`env::api::member`）/ 预算的取数判据只有这一处：`src/capabilities/runtime/service/env.rs` 的三个薄入口与建会话、回档、代理三条调用链都跟着它；改取数必须同改 `src/tests/workspace/resolve.rs`、`src/tests/runtime.rs` 的断言。
- `src/capabilities/turn/service/rebuild.rs` → 重建与代理装配的行为契约（与实时逐条一致）：改一处必须同改 `src/tests/api/compaction.rs`、`src/tests/collab/cards.rs`、`src/tests/session/rewind.rs`、`src/tests/runtime/proxy.rs` 的断言。
- `src/capabilities/turn/service/admit.rs` → 准入、按需重建、派发闸与子树续跑的唯一实现：`runtime` 的启用、派发与协作推进都经它，改语义必须同改 `src/tests/turn.rs` 的两条用例。
- `src/capabilities/turn/service/state.rs` → 访问器是唯一写点；新增状态块要同时说明它属于谁、由谁写（R4）。
- `src/capabilities/turn/service/turn.rs` / `flow.rs` → 回合推进与协作推进的行为契约（事件顺序、拒绝与提示文案）：改一处必须同改 `src/tests/runtime.rs`、`src/tests/collab/*` 的断言。
- `src/capabilities/turn/service/generation*` → 五条生成链的线程名、事件外送与落盘时机是行为契约：改一处必须同改 `src/tests/api/events.rs`、`src/tests/runtime.rs`、`src/tests/api/compaction.rs` 的断言。
- `src/capabilities/turn/api.rs` 的 `TurnHost` → 新加入口必须同改 `runtime` 的 `impl TurnHost for Handle`；它在 P5-9b 随命令队列手柄进 `src/entry/`。
- `src/tests/turn.rs` → 五条用例钉住运行态与创建占位的相互独立、shell 槽按会话共用、准入的落盘与进表、派发闸按运行态拒绝：改语义必须同改断言。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
