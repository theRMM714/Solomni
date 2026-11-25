# 单元地图 · session

> 定位：session（会话）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-24｜依据：`src/units/conversation/session/api.rs`

> 本文是 **session** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/conversation/session/detail/fs_history.rs` | `HistoryStore` 的落盘实现：`meta.yaml` + `transcript.jsonl`；子会话落在父会话的 `children/` 下 |
| `src/units/conversation/session/api.rs` | 入站能力面：`HistoryOps` / `History` 与会话事实、事件词汇、一席装配材料（`SessionPorts` / `MemberSpec` / `SingleSpec` / `MemberEnvParts` / `RebuildSpec` / `ProxySpec`）与核准规格（`VerifySpec`）的重导出 |
| `src/units/conversation/session/service.rs` | 用例与端口持有者：历史落盘端口只在这里（R12）；增量落盘手柄 `Persister` 与事件收编 `record_events`（核心操作回路在 `agentloop`） |
| `src/units/conversation/session/service/assemble.rs` | 一席的装配：能力面 `SessionPorts`、取来的本席事实（`MemberSpec` / `SingleSpec` 与逐字段拷来的 `MemberEnvParts`）、调用 `agentloop` 的装配机拼出成员工具面，以及三种在世形态的整场入口（`single_session` / `rebuild_single` / `proxy_session`） |
| `src/units/conversation/session/service/verify.rs` | 核实用例环境：按 `VerifySpec` 装配一个只读核实用例 `CoreVerify`（席位面、放行名单、围栏）；调用方给沙箱，本单元不取事实 |
| `src/units/conversation/session/service/tree.rs` | 会话树派生：子树、工作根、配置归属、树级装配 meta、guest 作用域与租约、树序与扁平模块 |
| `src/units/conversation/session/ports.rs` | 出站端口：`HistoryStore`（会话历史的读写与删除） |
| `src/units/conversation/session/domain/decisions.rs` | 裁决队列：先来后到、队首可见可答、回答校验、整队作废；对外只出窄端口 `Decisions`（会话表）与 `Gates`（一个会话的通道），队列结构与等待格不出单元 |
| `src/units/conversation/session/domain/seat_check.rs` | 席位面与工具总表的双向自洽断言（`problems`，工具面装配在 `agentloop`）：席位 id 唯一、提示词非空、核心席不授模块工具、能发布必须先能写、席位面工具都在总表里、机制与席位双向一致、`callers` 与席位面双向比对；声明本身在 `session_model` |
| `src/units/conversation/session/domain/flow.rs` | 流程面：驱动一侧的插件契约 `Flow`（推一步 / 取走让出的一步 / 交回处置 / 处置自己发的关卡）、让出的那一步 `MemberAsk`、交回后的处置 `Handback` 、关卡落定 `Settle` 与可派的活 `WorkPlan`（该派哪些节点、节点提示词与返工提示、开跑收工记账、开工事件）；**席位**自报在 `mechanism.rs` 的 `MECHANISMS`（每个机制的核心席与成员席各要几员）；不实现任何流程 |
| `src/units/conversation/session/domain/sequence.rs` | 复合流程：按会话类型表把多个流程串成一条（前一段 `is_done` 就把 `output` 交给下一段 `intake`）；驱动仍只认 `Flow`，业务面（`part`）指向当前这一段 |
| `src/units/conversation/session/domain/flows.rs` | 流程工厂面：`Flows`（id → 造一段）、装配用的 `FlowSetup`（会话参数、成员、裁决队与可选的恢复事实）与按类型表串段的 `sequence_of`；不实现任何流程，实现由组合根注入 |
| `src/units/conversation/session/detail/yaml_profiles.rs` | 会话类型表的磁盘实现：读 `systools/profiles.yaml`（缺文件 / 语法坏 / 规则不过都如实报），装载后交组合根 |