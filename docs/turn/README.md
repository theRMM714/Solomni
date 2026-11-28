# turn（回合驱动）

> 定位：回合驱动的唯一实现——会话准入与派发闸、按需重建、一席事实的取用、回合与协作的推进规则、五条生成链与点火器，以及事件台与逐轮落盘手柄。
> 效力：规范｜核对：2026-11-28｜依据：`src/units/business/turn/turn/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**会话准入**——`admit` 先落盘再进在世表，`ensure_session` 不在世就按转录重建，`dispatch_gate` 按运行态拒绝派发，`resume_subtree` 把停掉的子树拉回运行态（`src/units/business/turn/turn/service/admit.rs`）。
- 管：**按需重建**——`rebuild_session` 按落盘事件与元数据重建单 agent / 协作 / 代理三种形态，`workspace_binding_of` 做加载复检，`replay_dialogue` 还原对话与压缩边界，`build_proxy` 装配代理会话（`src/units/business/turn/turn/service/rebuild.rs`）。
- 管：**一席事实的取用**——沙箱清单、权限面、`env::api::member` 算出的工具环境（`MemberEnvParts`）与上下文预算（`src/units/business/turn/turn/service/facts.rs`）；事实都经各单元的 `api` 取，本单元不持端口。
- 管：**回合推进**——`single_say` / `continue_flow` / `identity_block` / `compact_plan` / `note_system`，以及生成前的工具形态刷新、前缀事件与 `Prepared`（`src/units/business/turn/turn/service/turn.rs`）。
- 管：**协作推进**——`collab_advance` / `collab_resume` / `collab_set_task` / `collab_answer` / `take_card` / `dispose_and_advance` / `open_queue` / `withdraw_agree` / `collab_pending`（`src/units/business/turn/turn/service/flow.rs`）。
- 管：**生成驱动**——单 agent / 节点 / 成员 / 协作泵 / 压缩五条链，以及七条点火器（节点执行、继续、代理回合、协作推进、裁决处置）；线程名 `solomni-*` 是行为契约的一部分，线程本身经宿主的 `TurnHost` 起。
- 管：**事件台** `EventBus`（`src/units/business/turn/turn/service/bus.rs`）——核心独占生产、按会话与序号增量取，回档后按会话重放；结构不出单元，只经 `EventFeed`（读）与 `Events`（写）两端。
- 管：**逐轮落盘手柄与运行态口径**（`src/units/business/turn/turn/service/history.rs`）——事件收编、`Persister` 取用与 `set_subtree_run`；落盘实现与目录布局归 `conversation::session`。
- 管：**调用参数与思考片段**（`src/units/business/turn/turn/service/model.rs`）——流式开关、超时、按登记处取思考等级；建会话与回合推进共用这一份判据。
- 不管：**在世表、运行态与创建占位、裁决队、流程契约**——归 `turn_state`（`Turn` / `Session` / `Gates` / `Flow` / `Sequence`），本单元只经它的 `api` 读写。
- 不管：**单 agent 会话对象与一席的装配**——`AgentSession`、工具面与三种在世形态的构造归 `agentloop`（`src/units/business/agentloop/domain/assembly.rs` 的 `single_session` / `rebuild_single` / `proxy_session`）；本单元取完事实、组 `MemberSpec` 后经 `Turn::build_single` 转调。
- 不管：**落盘与命令队列**——转录的读写归 `conversation::session`（`Persister` / `History`），谁在什么时候调生成入口、命令怎么排队由宿主 `runtime` 决定。
- 不管：登记处本身——工具模式、上下文预算与核心默认模型经调用方传进来的 `&dyn Registry` 现取。

## 入站契约与状态归属

- 入站面（命令）：`api::TurnOps`——`say` / `continue_flow` / `set_task` / `answer_card` / `open_queue` / `withdraw_agree` / `compact` / `is_running`；宿主（`runtime` 的 `Handle`）实现它，呈现层按屏幕只取这一面。
- 入站面（宿主端口）：`api::TurnHost`——业务循环自己做不了、必须宿主在核心线程上代做的入口（取放状态、事件台、guest 起停、裁决队与提问端口、生成登记、线程点火、回答裁决卡）；`api::session_ask` 是提问端口工厂。
- 入站面（生成与事件台）：`api::single_generation` / `api::collab_generation` / `api::compact`，七条点火器 `api::spawn_detached_*` / `api::start_child_collab`，`api::EventBus` 的读写两端与 `api::EventLine`，以及值类型 `Output` / `Advance` / `Prepared` / `CollabWork` / `AskReq` / `RestoreRequest`。
- 入站面（重建）：`api::replay_dialogue(&[Value], ToolMode) -> (对话, 标记, 回复号, 行号, 压缩截至行, 压缩截至回复)`、`api::PROXY_PRIVATE_DIR` / `api::SPEAKER`（代理会话的私有目录名与说话人，只有一处）。
- 状态归属：**事件台是它唯一自持的内存状态**（上限一万条、保留五千条）；会话在世表、运行态与创建占位在 `turn_state`，转录在 `conversation::session`，`Turn` 上的端口是构造时注入的借用方。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`agentloop`（`AgentSession` / `ShellSlot` / `SessionPorts` 与装配、重建入口）、`catalog`（`modules` 的 `Module` / `Roster`、`packages` 的 `PackagePlan`、`registry` 的 `Registry`、`secrets` 的 `SecretOps`）、`collab`（`CollabSession`）、`conversation`（会话转录与 `session` 子业务的落盘手柄）、`exec_model`（`Tier`）、`kernel`（`SessionId` / `Log` / `HostProbe`）、`llm`（`Llm` / `Channel` / `Msg`）、`permission`（`PermissionSet`）、`prompt`（`Prompt` / `Segment`）、`registry_model`（`AgentMeta`）、`session_model`（`SessionMeta` / `RunState` / `History`）、`tools`（`Tools` / `ToolExec`）、`transcript`（`ToolCallView` 与截断）、`turn_state`（`Decisions` / `Gates` / 流程面）、`vm`（`Vm` / `GuestOs`）、`workspace`（`Workspace` / `Sandbox`）；本单元内的 `env` 子业务直接调用，不经 `api`
- 谁在用我：`entry`、`presentation`、`runtime`、`work`

## 一、取出与放回

- 生成开始时取走会话对象（`Turn::take`）并标运行态：核心表里只留「它在跑」这个事实，长步骤因此不占队列。
- 生成在工作线程上推进，事件经 `EventBus` 外送、逐轮经 `Persister` 落盘；结束时放回（`Turn::put`）并撤运行态。**取放的时机仍由调用方决定**，生成期间的取 / 交经 `TurnHost::core` 回核心线程，好让别的命令照常排队。
- 会话进表前先落盘（`admit`）：名字即目录，落盘失败就不进表；再开时 `ensure_session` 从落盘重建，不保留进程内影子。

## 改动本单元时必须同步

- `src/units/business/runtime/service/core.rs` → `turn` 字段是唯一读写口（访问器在 `turn_state` 的 `Turn` 上，runtime 只转调）：增删访问器必须同改调用点与 [`unit-map.md`](unit-map.md)。
- `src/units/business/turn/turn/service.rs` → 分区声明；加删除模块必须同改本页与 [`unit-map.md`](unit-map.md)（T0 与磁盘双向比对）。
- `src/units/business/agentloop/domain/assembly.rs` → 装配顺序、条件与错误文案是行为契约（`build_single` 的形参变化必须同改 `src/units/business/work/service/create.rs`）：改判据必须同改 `src/tests/tools/shell.rs`、`src/tests/runtime.rs` 的断言；本单元只负责取事实与转调（`src/units/business/turn/turn/service/facts.rs`）。
- `src/units/business/turn/turn/service/facts.rs` → 沙箱 / 权限 / 工作区授权 / 工具环境（`env::api::member`）/ 预算的取数判据只有这一处：`src/units/business/runtime/service/env.rs` 的薄入口与建会话、回档、代理三条调用链都跟着它；改取数必须同改 `src/tests/workspace/resolve.rs`、`src/tests/runtime.rs` 的断言。
- `src/units/business/turn/turn/service/rebuild.rs` → 重建与代理装配的行为契约（与实时逐条一致）：改一处必须同改 `src/tests/api/compaction.rs`、`src/tests/collab/flow.rs`、`src/tests/session/rewind.rs`、`src/tests/runtime/proxy.rs` 的断言。
- `src/units/business/turn/turn/service/admit.rs` → 准入、按需重建、派发闸与子树续跑的唯一实现：`runtime` 的启用、派发与协作推进都经它，改语义必须同改 `src/tests/session/turn.rs` 的用例。
- `src/units/business/turn_state/domain/live.rs` / `src/units/business/turn_state/domain/turn.rs` → 访问器是唯一写点；新增状态块要同时说明它属于谁、由谁写（R4）。
- `src/units/business/turn/turn/service/turn.rs` / `flow.rs` → 回合推进与协作推进的行为契约（事件顺序、拒绝与提示文案）：改一处必须同改 `src/tests/runtime.rs`、`src/tests/collab/*` 的断言。
- `src/units/business/turn/turn/service/generation*` → 五条生成链的线程名、事件外送与落盘时机是行为契约：改一处必须同改 `src/tests/api/events.rs`、`src/tests/runtime.rs`、`src/tests/api/compaction.rs` 的断言。
- `src/units/business/turn/turn/api.rs` 的 `TurnHost` → 新加入口必须同改 `runtime` 的 `impl TurnHost for Handle`（命令队列手柄在 `src/entry/`）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
