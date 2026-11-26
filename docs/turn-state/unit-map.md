# 单元地图 · turn_state

> 定位：turn_state（回合状态与裁决机制）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/turn_state/api.rs`

> 表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)。
> 单元根 `src/units/business/turn_state.rs` 与 `domain.rs` 是骨架，不入本表。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/turn_state/api.rs` | 入站面：转出 `Turn` / `Session`、裁决队、流程契约与工厂、`session_ask`、`History` / `HistoryOps` 两个端口面；`domain` 不出单元，装配一席的 `SessionPorts` 随装配机在 `agentloop` |
| `src/units/business/turn_state/domain/ask.rs` | 裁决通道的回合侧实现：`AskUser`（推一条问题并阻塞等一个选项 id；空选项集回 `NoOptions` + `Notice`、不自停会话；没人答回 `NoAnswer` + `Notice`；停止 / 关闭按「拒绝」解开）与 `AskDesk` 的实现 `DeskFace`（`later` / `pending` / `head` / `drop_head` 四个窄方法转发给裁决队） |
| `src/units/business/turn_state/domain/decisions.rs` | 裁决队：队里存 `kernel::Ask`（`Entry { id, ask, slot }`），两个入口——`later` 不阻塞、可重建（沿用原卡号）与 `wait` 阻塞、线程绑定（不重建）；先来后到、队首可见可答、回答校验（卡号是队首 + 选项属于该卡）、整队作废（停止 = 拒绝）把等待方解开；对外出 `Gates` 与出队票 `GateTicket { option, note, ask }` |
| `src/units/business/turn_state/domain/flow.rs` | 流程面：插件契约 `Flow`（推一步 / 取走让出的一步 / 交回处置 / 处置自己发的关卡）、让出的那一步 `MemberAsk`、交回后的处置 `Handback`、关卡落定 `Settle` 与可派的活 `WorkPlan`；不实现任何流程 |
| `src/units/business/turn_state/domain/flows.rs` | 流程工厂面：`Flows`（id → 造一段）、装配用的 `FlowSetup`、按类型表串段的 `sequence_of`；实现由组合根注入 |
| `src/units/business/turn_state/domain/live.rs` | 在世表与运行态：`Session` 两种形态（单 agent / 一个流程槽）、`TurnState`（运行态、创建占位、裁决通道表、shell 活进程槽） |
| `src/units/business/turn_state/domain/sequence.rs` | 复合流程：按会话类型表把多个流程串成一条，前一段收尾就把产物交给下一段（`intake`），行号接上；驱动仍只认 `Flow` |
| `src/units/business/turn_state/domain/turn.rs` | 会话本体 `Turn`：持状态与端口，转发全部状态访问器，交出 `session_ports`，给 `running_refusal` 的唯一实现 |
