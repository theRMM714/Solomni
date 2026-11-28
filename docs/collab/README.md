# collab（协作会话）

> 定位：从讨论走到交付的协作会话状态机。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/business/collab/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：协作会话对象与状态（名单、方案、任务链、挂起）、讨论泵与轮转发言、整理出链、链驱动、节点与总验收及定向返工、断点续跑。
- 不管：不落盘（经 `session::api`）、不建会话（`runtime` 建）、不选模型（`registry` 解析）、不画界面（`cli` / `web`）；成员回合的驱动 / 轮循环 / 工具循环在 `agentloop` 的 `engine/`（见 [`docs/agentloop/unit-map.md`](../agentloop/unit-map.md)）；核实环境（只读核实用例）也由 `agentloop` 装配——本单元只按 `agentloop::api::VerifySpec` 递事实、拿回 `Option<Box<dyn CoreVerify>>` 并以 `Option<&mut dyn CoreVerify>` 传给 `agentloop` 的核心操作回路（`src/units/business/agentloop/domain/verify.rs`）。

## 入站契约与状态归属

- 入站面：`api::CollabSession`——讨论与执行引擎、回合与验收词汇、协作状态派生的对外名字；协作自己的事件（`CollabEvent`）与裁决关（`Pending`、`OPT_*`）也从 `api` 出。
- 词汇归属：协作事件与关卡在 `src/units/business/collab/domain/`（`events.rs` / `gates.rs`），会话只提供「机制事件」通道与不解释 `kind` 的裁决队——改协作流程不改会话文件。
- 流程面：协作在 `src/units/business/collab/service/flow.rs` 实现 `session::api::Flow`（推一步 / 让出一回合 / 交回处置 / 处置自己发的关卡）与可派的活 `session::api::WorkPlan`（该派哪些节点、节点提示词与返工提示、开跑收工记账、开工事件）；`turn` 的驱动只认流程面，认不出这是协作。关卡的解释在 `src/units/business/collab/domain/gates.rs`（`Pending::from_gate` 认出是哪一关，`CollabSession::dispose` 落定）。
- 两条流程：讨论（`discuss`：核心代理主持、讨论席开会，收敛后由核心写一份总结）与执行（`execute`：核心拟计划与任务链、执行席逐节点干、核心逐阶段验收后交付）。两者共用同一个 `CollabSession` 类型与同一份落盘事实，用 `Phase`（`src/units/business/collab/service/collab.rs`）分开；装配期由 `Sequence` 串起来——前一段交出的总结就是后一段的需求。
- 流程工厂：`src/units/business/collab/service/flows.rs` 的 `CollabFlows` 实现 `turn_state::api::Flows`（按 id 造讨论段或执行段，新建走 `start`、重建走 `restore`；重建只造段进度事实点名的那段之后的段，当前段只拿切点之后的转录窗口）；组合根把它注入 `turn` / `work`，装配点只按会话类型表的 `flows` 要段，因此 `work` / `turn` 不认识协作。
- 状态归属：会话对象及其状态只有它写；「转录即状态」的派生在 `src/units/business/collab/domain/collab_state.rs`（纯函数、可回放）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`agentloop`、`catalog`、`kernel`、`llm`、`prompt`、`registry_model`、`session_model`、`taskchain`、`tools`、`transcript`、`turn_state`、`vm`、`workspace`
- 谁在用我：`lifecycle`、`presentation`、`proxy`、`runtime`、`turn`

## 改动本单元时必须同步

- 核实环境的规格（`VerifySpec` 的字段与构造）只有 `src/units/business/agentloop/domain/verify.rs` 一处：改字段必须同改 `src/units/business/collab/service/turn_io.rs` 的 `verify_env` 与 `src/tests/collab/` 的核实用例。
- 状态派生（`src/units/business/collab/domain/`）与驱动（`src/units/business/agentloop/engine/driver.rs`）必须同改；同步 `src/tests/collab/` 五份用例、`docs/collab/task-chain.md`、`docs/session/session-model.md`、`docs/tools/tools-and-roles.md`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`task-chain.md`](task-chain.md) | 审查关卡、任务链（依赖图）、子会话与验收 |
