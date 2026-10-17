# 单元地图 · session

> 本文是 **session** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/session/detail/fs_history.rs` | `HistoryStore` 的落盘实现：`meta.yaml` + `transcript.jsonl`；子会话落在父会话的 `children/` 下 |
| `src/capabilities/session/api.rs` | 入站能力面：`HistoryOps` / `History` 与会话事实、事件词汇的重导出 |
| `src/capabilities/session/service.rs` | 用例与端口持有者：历史落盘端口只在这里（R12） |
| `src/capabilities/session/ports.rs` | 出站端口：`HistoryStore`（会话历史的读写与删除） |
| `src/capabilities/session/domain/decisions.rs` | 裁决队列：先来后到、队首可见可答、回答校验、整队作废 |
| `src/capabilities/session/domain/session.rs` | 单 agent 会话：上下文、行 id、回合与回复簿记 |
| `src/capabilities/session/domain/tools.rs` | 成员工具面：本回合可调的工具、观察账本与沙箱 |
| `src/capabilities/session/domain/history.rs` | 会话元信息、运行态（`RunState`）与历史视图的内存形态 |
| `src/capabilities/session/domain/events.rs` | 呈现侧事件与裁决卡信封等线格式词汇（`SessionEvent` / `LineView` 等） |
| `src/capabilities/session/domain/rewind.rs` | 回档的行 / 事件算术（纯函数，不碰 IO） |
