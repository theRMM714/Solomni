# 单元地图 · collab

> 本文是 **collab** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/collab/api.rs` | 入站能力面：`CollabSession`（讨论与执行引擎、回合与验收词汇、状态派生的对外名字） |
| `src/capabilities/collab/service/collab.rs` | 协作会话状态机：名单 / 讨论 / 链 / 裁决队句柄与各关卡登记、处置、快照 |
| `src/capabilities/collab/service/pump.rs` | 讨论泵：把讨论推进一步、装配成员、行外送与增量（泵不自己调模型） |
| `src/capabilities/collab/service/turn_io.rs` | 回合收发：成员回复喂回状态机、取出待问一步、回答落到业务 |
| `src/capabilities/collab/service/slate.rs` | 代拟名单与确认、开工、断点续跑与终结判定 |
| `src/capabilities/collab/service/discussion.rs` | 讨论状态机：建组、轮转发言、表态判定、全员同意后交整理 |
| `src/capabilities/collab/service/synthesis.rs` | 整理讨论出方案与任务链、总验收清单与返工判定 |
| `src/capabilities/collab/service/round.rs` | 轮循环与行构造：请求装配、`converse_with`、`build_round_lines`（唯一构造函数） |
| `src/capabilities/collab/service/tool_loop.rs` | 成员回合的工具循环：声明面 → 放行 → 并发调度 → 执行 → 回填 |
| `src/capabilities/collab/service/driver.rs` | 单回合驱动：在一条 agent 会话上跑讨论席 / 节点 / 压缩（会话当参数的自由函数） |
| `src/capabilities/collab/domain/collab_state.rs` | 「转录即状态」派生：从事件流水纯函数地算出当前状态（可回放、可单测） |
