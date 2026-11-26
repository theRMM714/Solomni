# transcript（转录）

> 定位：纯领域——会话流水线那一层线格式与呈现词汇的唯一家：事件、转录行、裁决卡、上下文用量。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/business/transcript/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话事件 `SessionEvent`（含机制事件通道 `Push`）、转录行 `LineView` 与工具调用行 `ToolCallView`、裁决卡与信封（`DecisionCard` / `DecisionEnvelope` / `DecisionMessage` / `OptionView` / `DecisionWaiter` / `DecisionQueue` / `DecisionAnswer`）与把 `kernel::Ask` 翻译成卡 / 等待方的五个自由函数（`card_of` / `waiter_of` / `event_of` / `json_of` / `note_of`）、上下文用量 `ContextUsage`，以及它们的构造函数与判定（`idle` / `working` / `qualify` / `last_compaction` / `interrupted_note` / `stopped_note`）。
- 不管：会话状态与派生（在 `conversation` 的 `session` 子业务）；裁决排队与等待的机制（那里的 `src/units/business/turn_state/domain/decisions.rs`）；落盘与回放（那里的 `detail/` 与 `service/`）；前端渲染（`src/presentation`）；措辞与译文（`prompt`）。
- 不管：不碰 IO、不持端口、不持有状态；但它是**环 2**而不是环 1——转录行里嵌着别的单元的载荷值（见「为什么是独立单元」）。

## 入站契约与状态归属

- 入站面：`api`——重导出上述形状与构造函数。
- 出站端口：无（纯领域，不自持端口）。
- 状态归属：不持有状态；这些值由会话与 runtime 产生，经事件台与转录落盘。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`（`api::note` 的 `Note` / `Severity`、`api::types` 的 `Ask`）、`llm`（`TokenUsage`：`ContextUsage::from_tokens` 的入参）、`tools`（`ObservationFact` / `ShellFacts`：工具调用行的载荷）
- 谁在用我：`agentloop`、`collab`、`conversation`、`presentation`、`proxy`、`runtime`、`session_model`、`turn`、`turn_state`、`work`

## 一、为什么是独立单元

- 这些形状原先挂在会话单元的 events.rs（618 行）：不含 IO、状态与端口，却被会话、协作、回合、运行时与两个前端共同读取。留在环 3 等于让「所有人都在读的线格式」挂在某一条用例下，读它就要连会话一起依赖。
- 拆出来以后：线格式有唯一家，任何单元与适配层都只经 `units::business::transcript::api` 取；会话只负责**产生**这些值，不再替所有人保管形状。
- **为什么是环 2 而不是环 1**：行里嵌着别的单元的值——`ContextUsage::from_tokens` 收 `llm::api::TokenUsage`，`ToolCallView` 持有 `tools::api::ObservationFact` 与 `tools::api::ShellFacts`（后者又牵 `workspace` 的 `ReadFact`）。这些载荷没下沉环 1 之前，把它放进 `src/units/pure/` 会让环 1 反向依赖环 2（H1 判红）。载荷下沉是 P4（registry-model / llm 协议）与工具事实归位的活，做完之后本单元可以搬进 `src/units/pure/`、环号改 1。
- 与会话的边界：形状与构造在这里；「什么时候产生哪条事件」「怎么回放成状态」「怎么排队等待」在会话单元。改形状要同时看两边的联动（见下）。

## 改动本单元时必须同步

- 改行 / 事件形状（`LineView` 字段、`SessionEvent` 变体、`kind` 词）→ 会话的派生与回放（`src/units/business/agentloop/domain/session.rs`、`src/units/business/turn_state/domain/decisions.rs`、`src/units/business/agentloop/engine/`）、`collab` 的状态派生、`runtime` 的视图、`cli` / `web` 的渲染、`src/tests/session/`，以及 [session-model.md](../session/session-model.md)。
- 改裁决卡形状或选项 id 的语义 → 会话的裁决队列、各机制的关卡词汇（协作在 `src/units/business/collab/domain/`）与 [session-model.md](../session/session-model.md) 的「请用户裁决」。
- 改上下文用量字段 → 状态栏与压缩判定（`service/engine/compact`、`presentation` 的状态栏）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
