# 单元地图 · session_model

> 定位：session_model（会话身份与声明）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/session_model/api.rs`

> 本文是 **session_model** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/session_model.rs` | 单元根：声明 `api`、`domain` 与 `detail` |
| `src/units/business/session_model/api.rs` | 入站面：重导出元信息、历史视图、声明聚合 `SessionDecls` 与段 / 角色类型、工具来源端口、裁决通道端口与留档策略（其它单元与呈现层只准用这里） |
| `src/units/business/session_model/domain.rs` | 定义入口：声明 `artifact`、`history`、`profile`、`mechanism`、`decls`、`persist`、`tools`、`decisions` 八个概念子模块 |
| `src/units/business/session_model/detail.rs` | 定义入口：声明 `yaml_decls` |
| `src/units/business/session_model/detail/yaml_decls.rs` | 会话形状声明的装配输入：经 `Assets` 读 `systools/loops.yaml` 与 `systools/profiles.yaml`，解码成 `SessionDecls` 并做跨表校验（缺文件 / 语法坏 / 规则不过都如实报），装载后交组合根 |
| `src/units/business/session_model/domain/artifact.rs` | 流程交接的产物与种类：`Artifact`（带内容的 `Requirement` / `Summary`，组合子只搬运）与 `Kind`（不带内容的种类，段声明的 `accepts` / `produces` 用它） |
| `src/units/business/session_model/domain/history.rs` | 会话元信息 `SessionMeta`、运行态 `RunState`、累计用量 `UsageTotals`、工作台声明 `WorkspaceDecl`、代理授权 `Delegation` 与历史视图 `HistoryView` |
| `src/units/business/session_model/domain/profile.rs` | 会话类型表 `Profiles` / `Profile` / `SessionSpec` 的形状与装配期校验（含 `sources` 工具来源 id 与 `system_tools` 系统工具；段相关判据要段表，读 `loops` 入参）、内置类型 id |
| `src/units/business/session_model/domain/decls.rs` | 会话形状声明的聚合 `SessionDecls`（段表 `Loops` + 类型表 `Profiles`）：跨表规则（流程段必须是声明过的流程、产物交接、工具来源）只在这里校验，`spec_of` / `min_agents` / `known` 也以聚合为入口 |
| `src/units/business/session_model/domain/decisions.rs` | 裁决通道端口 `AskDesk`（`later` 不阻塞 / `pending` / `head` / `drop_head`）：会话只认这四个方法，实现与队列在 `turn_state`（`src/units/business/turn_state/domain/ask.rs` 的 `DeskFace`） |
| `src/units/business/session_model/domain/tools.rs` | 工具来源端口：`ToolSource`（来源 id + `offer(TurnCtx)`）、`TurnCtx`（这一回合的上下文：流程段、agent、能力组、会话类型、工作区；段与角色由装配期拍平进来源实例）、能力组与来源 id 常量 |
| `src/units/business/session_model/domain/mechanism.rs` | 流程段与角色**声明**（唯一来源是 `systools/loops.yaml`）：`Role` / `Cast` / `Loop`（含实现名 `builder`、抑制能力组 `deny`、要点头的动作 `ask`、接什么产物 `accepts`、交什么产物 `produces`）、`Loops` 表与按 id 取段 / 角色 / 角色位 / 流程清单 / 成员角色 / 最少人数 |
| `src/units/business/session_model/domain/workspace.rs` | 工作台声明 `WorkspaceDecl`（策略 id + 外部根 + 不透明参数）与缺省策略 id `DEFAULT_POLICY`（`versioned`）：会话只说策略与参数，`params` 原样搬值、不解释字段；策略清单与实现都在工作台单元，认不出的 id 在开会话时按「没有这个工作台策略」报出来 |
| `src/units/business/session_model/domain/access.rs` | 权限链派生 `permissions`：全局默认 → 根 ceiling → 每层 narrowing，子会话只能更窄；不整包收设置，只收 `PermissionDefaults` |
| `src/units/business/session_model/domain/persist.rs` | 留档策略：哪种会话留档、哪些事件算定稿（`PersistPolicy`）、系统会话识别与拟名单会话号 |
