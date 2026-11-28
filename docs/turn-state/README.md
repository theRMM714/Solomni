# turn_state（回合状态与裁决机制）

> 定位：回合状态与裁决机制——在世表与运行态、裁决队、流程插件契约与工厂、提问桥、落盘与队列两个端口面（环 2）。
> 效力：规范｜核对：2026-11-28｜依据：`src/units/business/turn_state/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话本体 `Turn`——在世会话表（`Session` 两种形态）、运行态与创建占位（`TurnState`）、每会话的裁决通道与 shell 活进程槽，以及装配一席要用的端口（`session_ports`）。
- 管：裁决队——一条通道的排队与等待（`Decisions` / `Gates`）、队首可见可答、回答校验、整队作废（停止 = 拒绝）、卡号发放与续号。
- 管：流程插件契约——`Flow`（推一步 / 取走让出的一步 / 交回处置 / 处置自己发的关卡）、`MemberAsk`、`Handback`、`Settle`、`WorkPlan`，以及按 id 造流程的工厂 `Flows` / `FlowSetup` 与串段器 `sequence_of` / `Sequence`。
- 管：提问桥 `session_ask`——工具层的 `AskUser` 端口实现：登记卡、外送并落盘、停会话都走该会话同一条道。
- 管：两个端口面 `History`（直连面，由会话单元实现）与 `HistoryOps`（队列面，由 runtime 的队列代理实现），以及在世会话的状态与流程契约。
- 不管：回合驱动与生成（准入、重建、生成线程在 `src/units/business/turn/`）。
- 不管：任何具体流程的状态机（协作在 `src/units/business/collab/`）。
- 不管：落盘实现（`src/units/business/conversation/session/service.rs` 与 `detail/`）、会话身份与声明（`src/units/business/session_model/`）、呈现与命令队列（`presentation` / `runtime`）。

## 入站契约与状态归属

- 入站面：`api`——转出 `Turn` / `Session`、裁决队、流程契约与工厂、`session_ask`、`History` / `HistoryOps`；`domain` 不出单元，装配一席的 `SessionPorts` 随装配机在 `agentloop`。
- 出站端口：无自有 IO 端口；`History` 由会话单元实现、`Flow` 由协作实现、`Flows` 由组合根注入——本单元只认契约。
- 状态归属：`Turn` 是**核心线程上唯一持有在世会话与运行态的地方**，`runtime` 与 `turn` 经它放取；`TurnState` 不出单元（宿主与测试只经 `Turn` 的访问器）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`agentloop`、`catalog`、`kernel`、`llm`、`prompt`、`registry_model`、`residents`、`session_model`、`tools`、`transcript`、`vm`、`workspace`
- 谁在用我：`collab`、`conversation`、`lifecycle`、`presentation`、`runtime`、`turn`、`work`

## 一、为什么是独立单元

- 回合状态与流程契约原本摊在 `turn`（4.4k 行）与 `conversation/session` 两处：`runtime`、`lifecycle`、`work` 三方都要读在世表与运行态，于是三个用例单元互相依赖、并反向依赖 `turn`。
- 拆出来以后：**状态与契约**在环 2，所有用例（`turn` / `runtime` / `lifecycle` / `work`）都只向下；「一个单元依赖多个单元」变成「大家都依赖同一份状态与契约」。
- 为什么是环 2 而不是环 1：持跨回合状态、等用户回答（`Condvar` + 队列）、按端口驱动（落盘与裁决），不满足 H2 的「无 IO、无状态、无端口」。
- 为什么不把驱动一起搬：驱动要认识生成线程、命令队列与事件台（`turn` / `runtime` 的用例），搬进来会让环 2 反向依赖环 3。

## 改动本单元时必须同步

- 改 `Turn` 的字段或访问器 → `src/units/business/turn/turn/service/{facts,admit,rebuild,turn,flow}.rs`（直取点）、`src/units/business/runtime/service/core.rs`、`docs/turn/README.md`。
- 改 `Flow` 契约 → 协作各流程实现（`src/units/business/collab/service/`）、`src/units/business/turn/turn/service/flow.rs`、`docs/collab/README.md`。
- 改裁决队 → `src/units/business/runtime/service/handle.rs`、`src/presentation/cli/gate.rs`、`docs/session/session-model.md` 的「请用户裁决」。
- 改 `History` / `HistoryOps` → 会话单元的实现（`src/units/business/conversation/session/service.rs`）、`src/units/business/runtime/service/contracts.rs`、`src/presentation/faces.rs`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
