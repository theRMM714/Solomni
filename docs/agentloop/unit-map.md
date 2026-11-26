# 单元地图 · agentloop

> 定位：agentloop（单会话运行时）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/agentloop/api.rs`

> 本文是 **agentloop** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/agentloop.rs` | 单元根：声明 `api` / `domain` / `engine` 三个入口 |
| `src/units/business/agentloop/api.rs` | 入站面：重导出会话对象与回合事实、面装配与整场装配（含 `SessionPorts` 与三种在世形态的入口）、核实环境、回档算术、消息回灌与核心操作回路 |
| `src/units/business/agentloop/domain.rs` | 定义入口：声明 `assemble` / `assembly` / `member_tools` / `ports` / `reply` / `rewind` / `session` / `verify` 八个概念子模块 |
| `src/units/business/agentloop/domain/session.rs` | 单 agent 会话运行时：上下文、行 id、回合与回复簿记、压缩与回档、观察账本与新鲜度判定 |
| `src/units/business/agentloop/domain/session/prompt.rs` | 系统提示拼装：agent 与席位两条身份入口、模块提示、回合参数、环境块 |
| `src/units/business/agentloop/domain/member_tools.rs` | 成员工具面：本回合可调的工具、观察账本与沙箱；只含值的 `ToolFace` 与核实用例端口 `CoreVerify`；shell 状态槽与按转录派生最近一次 shell（`last_shell`） |
| `src/units/business/agentloop/domain/assemble.rs` | 工具面的**唯一装配机**：`assemble(FaceInput) -> SeatFace` 按席位现拼（席位需求展开 → 总表过滤 → 模块表与服务工具 → 说明块 → 发布工具执行者），另含纯派生 `face_tools` / `service_ops_of` / `serviced_tools` 与席位面展开 `seat_face` / `resolve` |
| `src/units/business/agentloop/domain/assembly.rs` | 一席的整场装配：参数对象 `MemberEnvParts` / `MemberSpec` / `SingleSpec` / `RebuildSpec` / `ProxySpec`，`member_tools` 造工具面活对象（**只在本单元内构造与持有**），以及 `single_session` / `rebuild_single` / `proxy_session` 三个在世形态入口 |
| `src/units/business/agentloop/domain/verify.rs` | 核实用例环境：按 `VerifySpec` 装配一个只读核实用例 `CoreVerify`（席位面、放行名单、围栏）；调用方给沙箱，本文件不取事实 |
| `src/units/business/agentloop/domain/ports.rs` | 装配一席的端口参数对象 `SessionPorts`（llm / 日志 / 提示词册 / 工具总表 / 工具执行 / 虚拟机 / 常驻服务 / 工作区 / 压缩通道）；与装配机同单元，否则环 2 内成环 |
| `src/units/business/agentloop/domain/rewind.rs` | 回档的行 / 事件算术（纯函数，不碰 IO）：回合归属、回复边界、行 id 推进、截断与保留 |
| `src/units/business/agentloop/domain/reply.rs` | 消息回灌：一次模型回复与工具结果 → 发给模型的消息序列（实时与重建的唯一来源） |
| `src/units/business/agentloop/engine.rs` | 引擎门面：声明 `core_op` / `decls` / `driver` / `fence` / `io` / `round` / `tools` 七个子模块 |
| `src/units/business/agentloop/engine/driver.rs` | 单回合驱动：在一条 `AgentSession` 上跑发言 / 派发 / 讨论席回合 / 继续 / 压缩（五个 `&mut self` 方法，取消与短暂事件出口按形参传） |
| `src/units/business/agentloop/engine/core_op.rs` | 核心操作回路：在工具面上跑一次核心操作（取载荷、走只读核实、把事实推给调用方），取消与分片的包装只在这里 |
| `src/units/business/agentloop/engine/decls.rs` | 工具声明面的值类型：`WireTools` / `Decls` / `ToolDecls`（`round` 与 `tools` 共用一份） |
| `src/units/business/agentloop/engine/io.rs` | 回合 IO 与结论：`TurnIo`（调用参数 / 裁决队 / 提问端口）与 `MemberTurn` / `CompactedTurn` |
| `src/units/business/agentloop/engine/round.rs` | 轮循环与行构造：请求装配 `assemble`、`converse_with`、`build_round_lines`（唯一构造函数）、轮 / 回复词汇 |
| `src/units/business/agentloop/engine/fence.rs` | 成员回合的写入门禁发布口与不透明写者采纳：`work_commit` 拒绝未读受管路径、`shell` / 模块工具落地后按批前观察事实生成有界回执 |
| `src/units/business/agentloop/engine/tools.rs` | 成员回合的工具循环：声明面 → 放行 → 并发调度 → 执行 → 回填 |
