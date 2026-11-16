# 单元地图 · session

> 定位：session（会话）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-17｜依据：`src/capabilities/session/api.rs`

> 本文是 **session** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/session/detail/fs_history.rs` | `HistoryStore` 的落盘实现：`meta.yaml` + `transcript.jsonl`；子会话落在父会话的 `children/` 下 |
| `src/capabilities/session/api.rs` | 入站能力面：`HistoryOps` / `History` 与会话事实、事件词汇的重导出 |
| `src/capabilities/session/service.rs` | 用例与端口持有者：历史落盘端口只在这里（R12）；核心操作回路 `core_operation`、增量落盘手柄 `Persister` 与事件收编 `record_events` |
| `src/capabilities/session/service/engine.rs` | 回合引擎门面：声明 driver / io / round / tools 四个子模块 |
| `src/capabilities/session/service/engine/driver.rs` | 单回合驱动：在一条 `AgentSession` 上跑发言 / 派发 / 讨论席回合 / 继续 / 压缩（五个 `&mut self` 方法，取消与短暂事件出口按形参传） |
| `src/capabilities/session/service/engine/io.rs` | 回合 IO 与结论：`TurnIo`（调用参数 / 裁决队 / 提问端口）与 `MemberTurn` / `CompactedTurn` |
| `src/capabilities/session/service/engine/round.rs` | 轮循环与行构造：请求装配 `assemble`、`converse_with`、`build_round_lines`（唯一构造函数）、轮 / 回复词汇 |
| `src/capabilities/session/service/engine/tools.rs` | 成员回合的工具循环：声明面 → 放行 → 并发调度 → 执行 → 回填 |
| `src/capabilities/session/service/tree.rs` | 会话树派生：子树、工作根、配置归属、树级装配 meta、guest 作用域与租约、树序与扁平模块 |
| `src/capabilities/session/ports.rs` | 出站端口：`HistoryStore`（会话历史的读写与删除） |
| `src/capabilities/session/domain/decisions.rs` | 裁决队列：先来后到、队首可见可答、回答校验、整队作废；对外只出窄端口 `Decisions`（会话表）与 `Gates`（一个会话的通道），队列结构与等待格不出单元 |
| `src/capabilities/session/domain/session.rs` | 单 agent 会话：上下文、行 id、回合与回复簿记、压缩与回档 |
| `src/capabilities/session/domain/session/prompt.rs` | 系统提示拼装：agent 与角色两条身份入口、模块提示、回合参数、环境块 |
| `src/capabilities/session/domain/tools.rs` | 成员工具面：本回合可调的工具、观察账本与沙箱；只含值的 `ToolFace` 与核实用例端口 `CoreVerify`；shell 状态槽与按转录派生最近一次 shell（`last_shell`） |
| `src/capabilities/session/domain/persist.rs` | 落盘策略：哪种会话留档、哪些事件算定稿（`PersistPolicy`）、系统会话的识别与拟名单会话号 |
| `src/capabilities/session/domain/history.rs` | 会话元信息、运行态（`RunState`）与历史视图的内存形态 |
| `src/capabilities/session/domain/events.rs` | 呈现侧事件与裁决卡信封等线格式词汇（`SessionEvent` / `LineView` 等） |
| `src/capabilities/session/domain/rewind.rs` | 回档的行 / 事件算术（纯函数，不碰 IO） |
