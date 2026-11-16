# turn（回合状态）

> 定位：回合状态的唯一持有者——会话在世表、运行态与创建占位、每会话的裁决队与 shell 活进程槽；会话本体与一席的装配材料也在这里收拢。
> 效力：规范｜核对：2026-11-16｜依据：`src/capabilities/turn/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**会话在世表**——键是会话名，值是一份在世会话的两种形态（单 agent / 协作）；生成期间对象被取出，表里不留影子。
- 管：**运行态与创建占位**——正在生成的会话集合、正在创建 / 启动的会话集合；改配、回档、删除据此拒绝。
- 管：**裁决队**——全会话一条裁决通道表（`session` 的 `Decisions`），同会话一次只问一条。
- 管：**每会话的 shell 活进程槽**——同一会话的多次 shell 调用共用一个活进程（`session::api::ShellSlot`）。
- 管：**会话本体** `Turn`——状态 + 装配要的能力面（`workspace` / `vm` / `residents` / `secrets` / `tools` / `systools` / `prompt` / `llm` / `log` / `compact` / `base_roots`），以及 `TurnState` 全部访问器的一行转发与 `running_refusal` 的唯一实现。
- 管：**一席的装配**——`member_tools` 把取来的本席事实装配成 `MemberTools`（工具面 + 执行者 + 提示行 + 隐私注入项），`single_session` 造单 agent 会话对象；纯派生在 `src/capabilities/turn/domain/assemble.rs`。
- 不管：会话对象（`session` 的 `AgentSession`、`collab` 的 `CollabSession`）怎么长、怎么跑——本单元只放取与标记。
- 不管：落盘与生成驱动——谁在什么时候取走会话、生成完怎么放回，由调用方（`conductor` 的用例）决定。
- 不管：登记处与设置（`registry`）——工具模式、上下文预算与核心默认模型由调用方算好放进 `MemberSpec` / `SingleSpec`；本单元不碰登记处。

## 入站契约与状态归属

- 入站面：`api::TurnState`（`session` / `session_mut` / `holds` / `put` / `take` / `is_running` / `mark_running` / `unmark_running` / `running_iter` / `sessions_iter` / `is_starting` / `mark_starting` / `unmark_starting` / `desk` / `shell_slot` / `shell_view`）、`api::Session`，以及 `api::Turn`（同样的访问器 + `set_base_roots` / `session_deps` / `compact_factory` / `running_refusal`）；别的业务与测试只经 `api` 取用（R1）。
- 入站面（装配）：`api::member_tools(&SessionDeps, &MemberSpec) -> MemberTools`、`api::single_session(&SessionDeps, &SingleSpec) -> (AgentSession, Vec<SessionEvent>)`，以及 `SessionDeps` / `MemberSpec` / `SingleSpec` 三个纯数据面；`conductor` 只负责取事实与转调。
- 状态归属：**它自己写，别人只经 `api` 取**（R4）——在世表、运行态、创建占位、裁决队与 shell 槽只有这里增删；能力面是构造时注入的借用方，`Turn` 不拥有它们的内部状态。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`session`（`AgentSession` / `ShellSlot` / `ShellLive` / `Decisions` / `decisions` / `ModuleTools` / `ServiceOp` / `ToolFace` / `MemberTools` / `SessionParams` / `SessionEvent` / `AgentMeta` / `CompactChatFactory` / `service_ops_for_modules` / `tool_table`）、`collab`（`CollabSession`）、`kernel`（`SessionId` / `Tier` / `Log`）、`workspace`（`Workspace` / `Sandbox`）、`tools`（`Tools` / `ToolExec` / `ToolBook` / `tool_notes`）、`prompt`（`Prompt` / `Segment`）、`permission`（`PermissionSet`）、`env`（`member` / `MemberFacts`）、`modules`（`Module`）、`packages`（`PackagePlan`）、`residents`（`ResidentOps` / `ServiceView` / `ServiceState`）、`secrets`（`SecretOps`）、`vm`（`Vm` / `GuestOs`）、`llm`（`Llm` / `Channel` / `ToolMode`）；登记处的值（工具模式、上下文预算、核心默认模型）由调用方取，本单元不持有它
- 谁在用我：`conductor`（服务与命令门面都经 `self.turn.*`，装配经 `turn::api::member_tools` / `single_session`）、`src/tests/turn.rs`

## 一、取出与放回

- 生成开始时取走会话对象（`take`）并标运行态：核心表里只留「它在跑」这个事实，长步骤因此不占队列。
- 生成结束时放回（`put`）并撤运行态；**放回的时机与事件顺序由调用方决定**，本单元不落盘、不发事件。
- 运行态与创建占位是两套集合：前者挡改配 / 回档 / 删除，后者挡同名并发创建。

## 改动本单元时必须同步

- `src/capabilities/conductor/service.rs` → `turn` 字段是唯一读写口（访问器在 `Turn` 上，conductor 只转调）：增删访问器必须同改调用点与 [`unit-map.md`](unit-map.md)。
- `src/capabilities/turn/service.rs` / `session.rs` → `Turn::new` 的能力面与 `Conductor::new` 的注入列表一一对应：加一处必须同改另一处，且 `session_deps` 要一并给出。
- `src/capabilities/turn/service/assemble.rs` → 装配顺序、条件与错误文案是行为契约（等价于原 `src/capabilities/conductor/service/env.rs`）：改判据必须同改 `env.rs` 的薄入口与 `src/tests/tools/shell.rs`、`src/tests/conductor.rs` 的断言。
- `src/capabilities/turn/service/state.rs` → 访问器是唯一写点；新增状态块要同时说明它属于谁、由谁写（R4）。
- `src/tests/turn.rs` → 三条用例钉住运行态与创建占位的相互独立、shell 槽按会话共用：改语义必须同改断言。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
