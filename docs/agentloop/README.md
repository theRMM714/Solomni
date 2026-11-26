# agentloop（单会话运行时）

> 定位：一条 agent 会话的运行时——会话状态与簿记、成员工具面与面装配、回合引擎、核心操作回路（环 2）。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/business/agentloop/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：单 agent 会话运行时 `AgentSession`——上下文、行 id、回合与回复簿记、压缩与回档、观察账本与新鲜度判定；回合参数 `SessionParams` 与提示拼装。
- 管：成员工具面 `ToolFace` / `MemberTools` 与核实端口 `CoreVerify`，以及工具面的**唯一装配机**（席位 → 面）；`last_shell` 这类按转录派生的运行事实。
- 管：回合引擎——五个回合入口（发言 / 派发 / 讨论席回合 / 继续 / 压缩）、轮循环与行构造、工具循环与放行策略、回合端口与结论类型；核心操作回路 `core_operation`。
- 管：回档的行 / 事件算术与消息回灌（`reply_msgs` / `tool_result_body`）。
- 不管：会话身份与声明（元信息、类型表、机制席位、留档策略在 `src/units/business/session_model/`）。
- 不管：历史落盘（`src/units/business/conversation/session/detail/`）、裁决队列与流程面（`src/units/business/turn_state/domain/`）、会话树派生（`src/units/business/conversation/session/service/tree.rs`）、呈现与命令队列（`presentation` / `runtime`）。
- 不管：模型通道与工具机制（在 `llm` / `tools` / `workspace` / `vm` / `residents` 各自端口后）。

## 入站契约与状态归属

- 入站面：`api`——重导出上述形状与函数（其它单元与呈现层只准用这里）。
- 出站端口：无自有端口；引擎按调用方注入的 `Chat` / `Tools` / `Llm` / `Vm` / `Workspace` / `ResidentOps` / `Prompt` 干活。
- 状态归属：`AgentSession` 是**一条会话的运行态**，活在会话表里（`turn` 持有并驱动它）；本单元不持有跨会话状态。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`catalog`、`exec_model`、`kernel`、`llm`、`prompt`、`registry_model`、`residents`、`session_model`、`tools`、`transcript`、`vm`、`workspace`
- 谁在用我：`collab`、`conversation`、`lifecycle`、`presentation`、`runtime`、`turn`、`turn_state`、`work`

## 一、为什么是独立单元

- 会话单元原有 9.2k 行，其中会话运行时（会话对象 1.1k、工具面 561、面装配 209、回合引擎约 2.6k、核心操作回路 330）与**用例层**（历史落盘、会话树、裁决队、流程面、回档编排）混在一起：读一条元信息、跑一个回合、写一次历史，全都指向同一个单元，扇出 17。
- 拆出来以后：**运行时**只向下依赖机制单元（模型、工具、工作区、席位声明），**用例**留在会话单元向下调用运行时；`turn` / `work` / `runtime` 不再为了驱动一个回合而拖进历史落盘与裁决队。
- 为什么是环 2 而不是环 1：它按端口驱动 IO（`Chat` / `Tools` / `Vm` / `Workspace`），持有跨回合会话状态，不满足 H2 的「无 IO、无状态」。
- 为什么不把裁决队与流程面一起搬：`flow` / `sequence` 依赖 `GateTicket` / `MemberTurn`，搬进运行时会让运行时的依赖面横跨「流程编排」，而流程编排正是用例层的职责。

## 改动本单元时必须同步

- 改 `AgentSession` 的字段或簿记方法 → `docs/session/session-model.md`、`src/units/business/turn/`（驱动点）、`src/tests/session/`。
- 改工具面装配（`assemble` / `face_of` / `sources`）→ 会话单元的席位自洽断言（`src/units/business/conversation/session/domain/seat_check.rs`）、`docs/tools/tools-and-roles.md`。
- 改回合引擎的端口或结论类型 → `src/units/business/turn/turn/service/`、`src/tests/tools/`、`docs/session/session-model.md`。
- 改核心操作回路 → `docs/tools/tools-and-roles.md`、`src/units/business/collab/` 与 `src/units/business/conversation/slate/` 的调用点。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
