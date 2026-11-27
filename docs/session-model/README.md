# session_model（会话身份与声明）

> 定位：会话的身份与声明——元信息、类型声明、流程段与角色、工具来源端口、留档策略的唯一家（环 2）。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/business/session_model/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话元信息 `SessionMeta`（含 `UsageTotals` / `WorkspaceBinding` / `RunState` / `Delegation`）与历史视图 `HistoryView`、默认的新鲜度保留比例 `default_freshness_keep_percent`。
- 管：会话类型表 `Profiles` / `Profile` / `SessionSpec` 与装配期校验、内置类型的 id、`sources` / `system_tools`。
- 管：流程段与角色**声明**——`Role` / `Cast` / `Loop` 三种形状（`Loop.builder` 声明这一段由哪个流程实现跑、`Loop.deny` 声明抑制哪几组能力、`Loop.ask` 声明哪些动作要用户点头、`Loop.accepts` / `Loop.produces` 声明收哪一种交接产物、交哪一种）、`Loops` 表（唯一来源 `systools/loops.yaml`）与按 id 取段 / 角色 / 角色位 / 流程清单 / 成员角色 / 最少人数。
- 管：两份声明的聚合 `SessionDecls`（段表 + 类型表）：**跨表**规则（流程必须是声明过的流程段、产物交接、工具来源）只在这里校验，`spec_of` / `min_agents` / `known` 以聚合为入口；装载在 `src/units/business/session_model/detail/yaml_decls.rs`。
- 管：工具来源端口 `ToolSource` 与回合上下文 `TurnCtx`（来源 id、能力组常量）。
- 管：留档策略 `PersistPolicy` / `persist_policy_for` / `is_system_session` / `SYSTEM_SID_SUGGEST`。
- 不管：会话对象与回合引擎（`src/units/business/agentloop/domain/session.rs` 与服务层）；角色面与工具总表的自洽断言（`src/units/business/conversation/session/domain/loop_check.rs`）；落盘 IO（`src/units/business/conversation/session/detail/`）；流程调度与回合推进（`collab` / `turn`）。

## 入站契约与状态归属

- 入站面：`api`——重导出上述形状、聚合 `SessionDecls` 与派生函数。
- 出站端口：无（不持端口，只由调用方按参数给）。
- 状态归属：不持有可变状态；`SessionMeta` 是会话落盘事实的内存形态，由会话单元的 `History` 读写。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`assets`（装载两份声明）、`exec_model`、`permission`、`registry_model`、`transcript`、`workspace`（历史里嵌着的档位、权限面与工作区绑定由这些单元定义，本单元只搬值）
- 谁在用我：`agentloop`、`collab`、`conversation`、`lifecycle`、`presentation`、`proxy`、`runtime`、`turn`、`turn_state`、`work`

## 一、为什么是独立单元

- 会话单元 9.6k 行里，`SessionMeta`（128 处引用）、`SessionSpec`（53 处）、`role` 与 `loop_id`、`RunState`（62 处）是全仓读得最多的形状；它们原先与回合引擎、裁决队列、工具面挤在同一个单元，导致 `turn` / `work` / `runtime` 为了读一条元信息而依赖整个会话实现。
- 拆出来以后：会话身份与声明有唯一家，读事实的单元只依赖这份声明，不再顺着会话对象向下拖。
- 为什么是环 2 而不是环 1（H2）：`SessionMeta.shared` 仍是 `workspace::api::SharedEdit`，`PersistPolicy` 认 `transcript::api::SessionEvent`；两处都在环 2。等 workspace 的共享区值模型与 transcript 的载荷下沉（H3）后，本单元可搬进 `src/units/pure/` 并改环 1。
- 为什么不连 `flow` / `flows` / `sequence` 一起搬：它们依赖裁决队（`GateTicket`、`Gates`）与回合 IO（`MemberTurn`、`TurnIo`），搬走会让本单元反向依赖会话单元，环内成环。

## 改动本单元时必须同步

- 改 `SessionMeta` 字段 → `docs/session/session-model.md`、会话 `meta.yaml` 读写、`src/tests/session/`。
- 改 `Profiles` / `SessionSpec` → `systools/profiles.yaml`、`src/units/business/session_model/domain/decls.rs`（跨表规则）、`docs/session/`、`src/tests/session/lifecycle.rs`。
- 改 `Role` / `Cast` / `Loop` / `Loops` → `systools/loops.yaml`（唯一声明处）、`src/units/business/conversation/session/domain/loop_check.rs`（自洽断言）、`src/units/business/agentloop/domain/sources.rs` 与 `src/units/business/agentloop/domain/policy.rs`（装配期按段拍平）、`docs/session/`。
- 改 `PersistPolicy` → 会话落盘判定与 `src/tests/session/`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
