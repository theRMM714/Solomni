# 单元地图 · session

> 定位：session（会话）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-28｜依据：`src/units/business/conversation/session/api.rs`

> 本文是 **session** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/conversation/session/detail/fs_history.rs` | `HistoryStore` 的落盘实现：`meta.yaml` + `transcript.jsonl`；子会话落在父会话的 `children/` 下 |
| `src/units/business/conversation/session/api.rs` | 入站面：`HistoryOps` / `History` 与会话事实、事件词汇、落盘手柄与会话树派生的重导出；一个身份装配材料随装配机在 `agentloop::api` |
| `src/units/business/conversation/session/service.rs` | 用例与端口持有者：历史落盘端口只在这里（R12）；增量落盘手柄 `Persister` 与事件收编 `record_events`（核心操作回路在 `agentloop`） |
| `src/units/business/conversation/session/service/tree.rs` | 会话树派生：子树、工作根、配置归属、树级装配 meta、guest 作用域与租约、树序与扁平模块 |
| `src/units/business/conversation/session/ports.rs` | 出站端口：`HistoryStore`（会话历史的读写与删除） |
| `src/units/business/conversation/session/domain/loop_check.rs` | 流程段与工具总表的自洽断言（`problems(loops, book)`，工具面装配在 `agentloop`）：角色 id 唯一、提示词非空，流程段 id 唯一、角色位引用的角色在册、角色位交的机制动作都在总表里，流程段要同时有核心与成员角色、静止段恰好一个角色位，`ask` 的动作在总表、`deny` 的能力组在册；声明本身在 `session_model` |