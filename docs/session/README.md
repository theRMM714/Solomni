# session（会话）

> 定位：会话状态、回合簿记与会话历史的唯一持有者。
> 效力：规范｜核对：2026-11-23｜依据：`src/capabilities/conversation/session/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话状态与簿记（行 / 回合 / 回复）、成员工具面与工具表、历史视图与元信息、回档的纯行算术、呈现侧事件契约（`SessionEvent` / `LineView` / 裁决关 `Gate`；机制专有事件走 `Push`，词汇归各自能力）、**流程面**（`Flow` 与可派的活 `WorkPlan`：驱动只认「推一步 / 让出一回合 / 交回处置 / 该派哪些节点」，具体流程自己实现）、**会话类型表**（`Profiles`：一条类型带 id / 显示名 / 一句说明、一串流程 id 与（没有流程时的）静止身份，声明在 `systools/profiles.yaml`；席位与人数由流程自己报，在 `session/domain/flow.rs` 的 `FLOWS`；装配期解析成 `SessionSpec` 落进 `meta`，之后装配只看声明，装配期校验）、核心操作回路（声明工具面 → 跑一次模型 → 取载荷 → 只读核实）。
- 不管：不编排跨会话（`runtime`）、不决定讨论流程（`collab`）、不实现工具机制（`tools`）。

## 入站契约与状态归属

- 入站面：`api::HistoryOps`（呈现层队列面）+ `api::History`（别的能力的直连面：造会话 / 写元信息 / **只读元信息** / 追流水 / 读回 / 删除）。
- 事件与运行态：行与事件词汇；**运行态** `RunState`（`active` / `stopped` / `archived`）落盘在 `meta.run`。
- 出站端口：`HistoryStore` **只由 `service.rs` 持有**（R12）。
- 状态归属：会话状态、回合簿记与会话历史由本单元唯一持有。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`modules`、`permission`、`prompt`、`residents`、`taskchain`、`tools`、`vm`、`workspace`
- 谁在用我：`collab`、`runtime`、`registry`、`slate`

## 改动本单元时必须同步

- 行与事件词汇（`src/capabilities/conversation/session/domain/events.rs`、`LineView` 字段）一改 → `collab` 的状态派生、`runtime` 的视图、`cli` / `web` 渲染、`src/tests/session/` 全跟。
- 机制自己的事件与关卡（`Push` 的 `kind` / `payload`、`Gate`）改 → 对应机制的词汇文件（协作在 `src/capabilities/conversation/collab/domain/events.rs` 与 `src/capabilities/conversation/collab/domain/gates.rs`）与它的渲染；会话侧不跟着改。
- 流程面（`src/capabilities/conversation/session/domain/flow.rs` 的 `Flow` / `MemberAsk` / `Handback` / `WorkPlan`）一改 → 每个流程的实现（协作在 `src/capabilities/conversation/collab/service/flow.rs`）与驱动（`src/capabilities/turn/turn/service/flow.rs`、`src/capabilities/turn/turn/service/generation/collab.rs`）一起改。
- 复合流程（`src/capabilities/conversation/session/domain/sequence.rs` 的 `Sequence`）一改 → 装配点（`src/capabilities/work/service/create.rs`、`src/capabilities/turn/turn/service/rebuild.rs`）与流程自己的 `intake` / `output` / `part` 一起看：前一段交出的产物就是后一段的需求。
- 流程工厂面（`src/capabilities/conversation/session/domain/flows.rs` 的 `Flows` / `FlowSetup` / `sequence_of`）一改 → 各流程工厂的实现（协作在 `src/capabilities/conversation/collab/service/flows.rs`）与组合根的注入（`src/capabilities/runtime/service/core.rs`）一起改：装配点只问 id 要段，不认识具体流程。
- 会话类型表（`src/capabilities/conversation/session/domain/profile.rs` 的 `Profiles`，数据在 `systools/profiles.yaml`）一改 → 提示词册子的角色条目（`prompts/shared/mechanisms.yaml`）同跟；新增流程要在 `session/domain/flow.rs` 的 `FLOWS` 加一行（席位与人数在那里），角色换席要同改 `systools/roles.yaml` 的 `seat`；临时拼装（省略 `kind`、内联流程 id）走同一份校验 `check_inline` 与折算函数 `inline_spec`，落盘类型 `custom`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`session-model.md`](session-model.md) | 主 / 子会话、回合、发言标记、回档同步、上下文压缩 |
