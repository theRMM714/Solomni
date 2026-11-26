# 单元地图 · session_model

> 定位：session_model（会话身份与声明）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/session_model/api.rs`

> 本文是 **session_model** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/session_model.rs` | 单元根：声明 `api` 与 `domain` |
| `src/units/business/session_model/api.rs` | 入站面：重导出元信息、历史视图、类型声明、机制席位表与留档策略（其它单元与呈现层只准用这里） |
| `src/units/business/session_model/domain.rs` | 定义入口：声明 `history`、`profile`、`mechanism`、`persist`、`tools` 五个概念子模块 |
| `src/units/business/session_model/domain/history.rs` | 会话元信息 `SessionMeta`、运行态 `RunState`、累计用量 `UsageTotals`、工作区绑定 `WorkspaceBinding`、代理授权 `Delegation` 与历史视图 `HistoryView` |
| `src/units/business/session_model/domain/profile.rs` | 会话类型表 `Profiles` / `Profile` / `SessionSpec` 的形状与装配期校验（含 `sources` 工具来源 id 与 `system_tools` 系统工具）、内置类型 id |
| `src/units/business/session_model/domain/tools.rs` | 工具来源端口：`ToolSource`（来源 id + `offer(TurnCtx)`）、`TurnCtx`（这一回合的上下文：角色、agent、能力组、会话类型、工作区）、能力组与来源 id 常量 |
| `src/units/business/session_model/domain/mechanism.rs` | 机制与席位**声明**：`Seat` / `Mechanism`、`SEATS` / `MECHANISMS` 两张表与按 id 取机制 / 席位 / 流程清单 |
| `src/units/business/session_model/domain/persist.rs` | 留档策略：哪种会话留档、哪些事件算定稿（`PersistPolicy`）、系统会话识别与拟名单会话号 |
