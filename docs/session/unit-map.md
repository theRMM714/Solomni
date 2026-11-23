# 单元地图 · session

> 定位：session（会话）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-23｜依据：`src/capabilities/conversation/session/api.rs`

> 本文是 **session** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conversation/session/detail/fs_history.rs` | `HistoryStore` 的落盘实现：`meta.yaml` + `transcript.jsonl`；子会话落在父会话的 `children/` 下 |
| `src/capabilities/conversation/session/api.rs` | 入站能力面：`HistoryOps` / `History` 与会话事实、事件词汇、一席装配材料（`SessionPorts` / `MemberSpec` / `SingleSpec` / `MemberEnvParts` / `RebuildSpec` / `ProxySpec`）与核准规格（`VerifySpec`）的重导出 |
| `src/capabilities/conversation/session/service.rs` | 用例与端口持有者：历史落盘端口只在这里（R12）；核心操作回路 `core_operation`、增量落盘手柄 `Persister` 与事件收编 `record_events` |
| `src/capabilities/conversation/session/service/engine.rs` | 回合引擎门面：声明 driver / io / round / tools 四个子模块 |
| `src/capabilities/conversation/session/service/engine/driver.rs` | 单回合驱动：在一条 `AgentSession` 上跑发言 / 派发 / 讨论席回合 / 继续 / 压缩（五个 `&mut self` 方法，取消与短暂事件出口按形参传） |
| `src/capabilities/conversation/session/service/engine/decls.rs` | 工具声明面的值类型：`WireTools` / `Decls` / `ToolDecls`（`round` 与 `tools` 共用一份） |
| `src/capabilities/conversation/session/service/engine/io.rs` | 回合 IO 与结论：`TurnIo`（调用参数 / 裁决队 / 提问端口）与 `MemberTurn` / `CompactedTurn` |
| `src/capabilities/conversation/session/service/engine/round.rs` | 轮循环与行构造：请求装配 `assemble`、`converse_with`、`build_round_lines`（唯一构造函数）、轮 / 回复词汇 |
| `src/capabilities/conversation/session/service/engine/fence.rs` | 成员回合的写入门禁发布口与不透明写者采纳：`work_commit` 拒绝未读受管路径、`shell` / 模块工具落地后按批前观察事实生成有界回执 |
| `src/capabilities/conversation/session/service/engine/tools.rs` | 成员回合的工具循环：声明面 → 放行 → 并发调度 → 执行 → 回填 |
| `src/capabilities/conversation/session/service/assemble.rs` | 一席的装配：能力面 `SessionPorts`、取来的本席事实（`MemberSpec` / `SingleSpec` 与逐字段拷来的 `MemberEnvParts`）、成员工具面的装配，以及三种在世形态的整场入口（`single_session` / `rebuild_single` / `proxy_session`） |
| `src/capabilities/conversation/session/service/verify.rs` | 核实用例环境：按 `VerifySpec` 装配一个只读核实用例 `CoreVerify`（角色面、放行名单、围栏）；调用方给沙箱，本单元不取事实 |
| `src/capabilities/conversation/session/service/tree.rs` | 会话树派生：子树、工作根、配置归属、树级装配 meta、guest 作用域与租约、树序与扁平模块 |
| `src/capabilities/conversation/session/ports.rs` | 出站端口：`HistoryStore`（会话历史的读写与删除） |
| `src/capabilities/conversation/session/domain/decisions.rs` | 裁决队列：先来后到、队首可见可答、回答校验、整队作废；对外只出窄端口 `Decisions`（会话表）与 `Gates`（一个会话的通道），队列结构与等待格不出单元 |
| `src/capabilities/conversation/session/domain/session.rs` | 单 agent 会话：上下文、行 id、回合与回复簿记、压缩与回档 |
| `src/capabilities/conversation/session/domain/session/prompt.rs` | 系统提示拼装：agent 与角色两条身份入口、模块提示、回合参数、环境块 |
| `src/capabilities/conversation/session/domain/assemble.rs` | 装配用的纯派生：按权限面过滤角色面的 `face_tools`、就绪服务的工具表 `service_ops_of`、服务参数与工具行的 `serviced_tools` |
| `src/capabilities/conversation/session/domain/tools.rs` | 成员工具面：本回合可调的工具、观察账本与沙箱；只含值的 `ToolFace` 与核实用例端口 `CoreVerify`；shell 状态槽与按转录派生最近一次 shell（`last_shell`） |
| `src/capabilities/conversation/session/domain/persist.rs` | 落盘策略：哪种会话留档、哪些事件算定稿（`PersistPolicy`）、系统会话的识别与拟名单会话号 |
| `src/capabilities/conversation/session/domain/history.rs` | 会话元信息、运行态（`RunState`）与历史视图的内存形态 |
| `src/capabilities/conversation/session/domain/mechanism.rs` | 机制面：驱动一侧的插件契约 `Mechanism`（推一步 / 取走让出的一步 / 交回处置）、让出的那一步 `MemberAsk` 与交回后的处置 `Handback`；不实现任何机制 |
| `src/capabilities/conversation/session/domain/profile.rs` | 会话类型表：`Profiles` / `Profile` / `MechDecl` 的形状与装配期校验（与形态一一对应、机制已注册、产物按顺序接得上、角色在册）；不解析 yaml，不实现机制 |
| `src/capabilities/conversation/session/detail/yaml_profiles.rs` | 会话类型表的磁盘实现：读 `systools/profiles.yaml`（缺文件 / 语法坏 / 规则不过都如实报），装载后交组合根 |
| `src/capabilities/conversation/session/domain/mode.rs` | 工作形态的词汇（`Mode`）：三个取值、与外部词的互转、落盘小写形态 |
| `src/capabilities/conversation/session/domain/events.rs` | 呈现侧线格式词汇：通用事件与机制事件通道 `Push`（`SessionEvent`）、裁决卡信封、裁决关 `Gate` 与出队票；不解任何机制的关卡 |
| `src/capabilities/conversation/session/domain/rewind.rs` | 回档的行 / 事件算术（纯函数，不碰 IO） |
