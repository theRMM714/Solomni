# 单元地图 · collab

> 定位：collab（协作会话）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-23｜依据：`src/capabilities/conversation/collab/api.rs`

> 本文是 **collab** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conversation/collab/api.rs` | 入站能力面：`CollabSession`（讨论与执行引擎、回合与验收词汇、状态派生的对外名字） |
| `src/capabilities/conversation/collab/service/collab.rs` | 协作会话状态机：名单 / 讨论 / 链 / 裁决队句柄与各关卡登记、处置、快照 |
| `src/capabilities/conversation/collab/service/pump.rs` | 讨论泵：把讨论推进一步、装配成员（泵不自己调模型） |
| `src/capabilities/conversation/collab/service/lines.rs` | 行外送与待裁决关的派生：行 → 带稳定 id 的事件、状态 → 整队待答的关、增量与审查事件 |
| `src/capabilities/conversation/collab/service/turn_io.rs` | 回合收发：成员回复喂回状态机、取出待问一步、回答落到业务；核实环境按 `session` 的 `VerifySpec` 递事实（`verify_env`），不自己造工具面 |
| `src/capabilities/conversation/collab/service/slate.rs` | 代拟名单与确认、开工、断点续跑与终结判定 |
| `src/capabilities/conversation/collab/service/discussion.rs` | 讨论状态机：建组、轮转发言、表态判定与协议反应（`AfterTurn`）；轮循环与工具循环已迁 session（见 [`docs/session/unit-map.md`](../session/unit-map.md)） |
| `src/capabilities/conversation/collab/service/synthesis.rs` | 整理讨论出方案与任务链、总验收清单与返工判定 |
| `src/capabilities/conversation/collab/domain/events.rs` | 协作的事件词汇：机制变体 → `SessionEvent::Push`、线上事件 → 机制变体，验收条目 `CheckView` 随它 |
| `src/capabilities/conversation/collab/domain/gates.rs` | 协作的关卡词汇：待决项种类、选项 id、卡片与背景文案，并翻译成 `session` 的 `Gate`（队列不认这些） |
| `src/capabilities/conversation/collab/service/flow.rs` | 协作作为流程：把泵的推进一步 / 让出成员回合 / 交回处置 / 关卡落定（`settle` / `settles_inline`）翻译成 `session` 的 `Flow`（驱动因此不认识协作） |
| `src/capabilities/conversation/collab/domain/collab_state.rs` | 「转录即状态」派生：从事件流水纯函数地算出当前状态（可回放、可单测） |
