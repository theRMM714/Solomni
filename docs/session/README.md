# session（会话）

> 定位：会话用例与编排——历史落盘、会话树与一席的装配（单会话运行时在 `agentloop`，裁决队与流程面在 `turn_state`）。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/business/conversation/session/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话历史的落盘与读写（实现 `turn_state` 的两个端口面 `History` / `HistoryOps`）、会话树派生（子树 / 工作根 / 配置归属 / guest 租约）、工具面与工具总表的自洽断言。
- 不管：`data/session` 这块盘本身归 [`sessions` 单元](../sessions/README.md)（会话根 + 存在性）；本单元只在它给的根下写 `meta.yaml` / `transcript.jsonl` / `children/`。
- 不管：不实现单会话运行时（会话对象、回合引擎、工具面、一个身份装配与核实环境、核心操作回路在 `agentloop`）、不持裁决队与流程面（在 `turn_state`）、不编排跨会话（`runtime`）、不决定讨论流程（`collab`）、不实现工具机制（`tools`）、不声明会话身份（元信息 / 类型表 / 流程段与角色在 `session_model`，本单元只按声明装配）。

## 入站契约与状态归属

- 入站面：本单元的 `api` 只出落盘手柄、会话树派生与角色面自洽断言（一个身份装配与核实环境随装配机在 `agentloop::api`）；`History` / `HistoryOps` 两个**端口面在 `turn_state::api` 定义**，本单元的 `service.rs` 与 `runtime` 的队列代理各实现一侧。
- 事件与运行态：行与事件词汇；**运行态** `RunState`（`active` / `stopped` / `archived`）落盘在 `meta.run`。
- 出站端口：`HistoryStore` **只由 `service.rs` 持有**（R12）。
- 状态归属：会话历史由本单元唯一持有；**一条会话的运行态**（上下文、行 id、回合与回复簿记）由 `agentloop` 的会话对象持有，本单元只持有它的入口。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`agentloop`、`catalog`、`exec_model`、`kernel`、`llm`、`prompt`、`registry_model`、`session_model`、`tools`、`transcript`、`turn_state`
- 谁在用我：`collab`、`lifecycle`、`runtime`、`turn`、`work`

## 改动本单元时必须同步

- 一条会话的运行时（`src/units/business/agentloop/domain/session.rs`）或回合引擎（`src/units/business/agentloop/engine/`）一改 → 装配入口（`src/units/business/agentloop/domain/assembly.rs`，在 `agentloop`）与驱动（`src/units/business/turn/`）同跟。
- 行与事件词汇（`src/units/business/transcript/domain/events.rs`、`LineView` 字段）一改 → `collab` 的状态派生、`runtime` 的视图、`cli` / `web` 渲染、`src/tests/session/` 全跟。
- 机制自己的事件与关卡（`Push` 的 `kind` / `payload`、`Ask` 的 `kind` / `payload`）改 → 对应机制的词汇文件（协作在 `src/units/business/collab/domain/events.rs` 与 `src/units/business/collab/domain/gates.rs`）与它的渲染；会话侧不跟着改。
- 流程面（`src/units/business/turn_state/domain/flow.rs` 的 `Flow` / `MemberAsk` / `Handback` / `WorkPlan`，归 `turn_state`）一改 → 每个流程的实现（协作在 `src/units/business/collab/service/flow.rs`）与驱动（`src/units/business/turn/turn/service/flow.rs`、`src/units/business/turn/turn/service/generation/collab.rs`）一起改。
- 复合流程（`src/units/business/turn_state/domain/sequence.rs` 的 `Sequence`）一改 → 装配点（`src/units/business/work/service/create.rs`、`src/units/business/turn/turn/service/rebuild.rs`）与流程自己的 `intake` / `output` / `part` 一起看：前一段交出的产物就是后一段的需求，货币是 `Artifact`（`src/units/business/session_model/domain/artifact.rs`），段声明里按 `Kind` 写 `accepts` / `produces` 并由 `Profiles::validate` 校验。
- 流程工厂面（`src/units/business/turn_state/domain/flows.rs` 的 `Flows` / `FlowSetup` / `sequence_of`）与注册表（`src/units/business/turn_state/domain/builder.rs` 的 `FlowBuilder` / `FlowBuilders`）一改 → 各流程的注册项（协作在 `src/units/business/collab/service/builders.rs`）与组合根的登记（`src/units/business/runtime/service/core.rs`）一起改：装配点只问 id 与段声明的实现名，不认识具体流程。
- 会话类型表（`src/units/business/session_model/domain/profile.rs` 的 `Profiles`，数据在 `systools/profiles.yaml`）一改 → `src/units/business/session_model/domain/decls.rs` 的跨表校验同跟、提示词册子的机制条目（`prompts/shared/mechanisms.yaml`，按角色 id 匹配）同跟；新增流程段要在 `systools/loops.yaml` 的 `loops` 加一条（实现名 `builder`、角色位与人数在那里），角色换提示词同改同一份 yaml 的 `roles`；`agent_visible` 决定 agent 能经 `create_session` 建哪些类型（默认都不可见，人的入口不受限）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`session-model.md`](session-model.md) | 主 / 子会话、回合、发言标记、回档同步、上下文压缩 |