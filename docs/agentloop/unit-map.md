# 单元地图 · agentloop

> 定位：agentloop（单会话运行时）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-28｜依据：`src/units/business/agentloop/api.rs`

> 本文是 **agentloop** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/agentloop.rs` | 单元根：声明 `api` / `domain` / `engine` 三个入口 |
| `src/units/business/agentloop/api.rs` | 入站面：重导出会话对象与回合事实、面装配与整场装配（含 `SessionPorts` 与三种在世形态的入口）、核实环境、消息回灌与核心操作回路 |
| `src/units/business/agentloop/domain.rs` | 定义入口：声明 `assemble` / `assembly` / `explain` / `member_tools` / `policy` / `ports` / `reply` / `session` / `sources` / `verify` 十个概念子模块 |
| `src/units/business/agentloop/domain/session.rs` | 单 agent 会话运行时：上下文、行 id、回合与回复簿记、压缩与回档、观察账本与新鲜度判定 |
| `src/units/business/agentloop/domain/session/prompt.rs` | 系统提示拼装：agent 与角色两条身份入口、模块提示、回合参数、环境块 |
| `src/units/business/agentloop/domain/member_tools.rs` | 成员工具面：本回合可调的工具、观察账本与执行面句柄 `exec: Arc<dyn sandbox::api::Sandbox>`（本机 / 客机由策略在装配期开出，`ToolFace` 不再带 `side` / `guest_lease` / 运行期路径）；工作台策略与它给的回合钩子 `driver: Option<Arc<dyn BenchDriver>>`（换席时随面重开）；只含值的 `ToolFace` 与核实用例端口 `CoreVerify`；shell 状态槽与按内容日志派生最近一次 shell（`last_shell`） |
| `src/units/business/agentloop/domain/assemble.rs` | 工具面的**唯一装配机**：`assemble(FaceInput) -> SeatFace` 造 `TurnCtx`、按会话类型挂的 `ToolSource` 逐个问 `offer`、去重成面，再按工作台策略的 `plan` 拿共享区动作与执行者（`FaceInput.policy` + `workspace_params`，本单元不认识版本化）；段与角色在这里按声明表拍平（`FaceInput.loops` 查出角色位的动作交给 `loop` 来源）；另含纯派生 `offers_of` / `merge_offers` / `face_of` / `service_ops_of` / `serviced_tools` |
| `src/units/business/agentloop/domain/explain.rs` | 工具面的解释（纯函数，不碰 IO、不执行）：按与装配同源的取数（来源逐个 `offer` 再合并）列出每个来源交了哪些 id、合并后的面、每个 id 的授权判定与理由（`Allow` / `Ask` / `Deny`，理由来自同一条段声明），以及面里存在但总表与模块工具表都没有的孤儿 id；给 `tools explain` 与人读诊断用 |
| `src/units/business/agentloop/domain/sources.rs` | `ToolSource` 的五个实现：`AgentSource`（按能力组交文件面与执行面）、`ModuleSource`（交限定 id `<模块>.<工具>`）、`LoopSource`（只持有装配期拍平好的角色动作 id，运行期不查段表）、`WorkspaceSource`（只交装配期由工作台策略给的共享区动作 id，运行期不判能力组）、`ProfileSource`（交会话类型声明的系统工具）；`build_sources` 按来源 id 造实例 |
| `src/units/business/agentloop/domain/policy.rs` | 授权端口：`Policy::decide`（准不准，结论 `Allow` / `Ask` / `Deny`）与实现 `LoopPolicy`（装配期从段表拷下 `deny` / `ask`，运行期不查表；先按 `deny` 拒、再按 `ask` 抬起提问；`LoopPolicy::none()` 是不抑制的默认）、能力组到工具 id 的映射 `ids_of`（与发放共用）、`ask_card` 造那一张 `kind = policy` 的卡；职责与「在不在场」分开 |
| `src/units/business/agentloop/domain/assembly.rs` | 一席的整场装配：参数对象 `MemberEnvParts` / `MemberSpec` / `SingleSpec` / `RebuildSpec` / `ProxySpec`，`member_tools` 造工具面活对象（**只在本单元内构造与持有**）、`open_exec` 按声明与事实从 `SandboxPolicies` 开执行句柄，以及 `single_session` / `rebuild_single` / `proxy_session` 三个在世形态入口 |
| `src/units/business/agentloop/domain/verify.rs` | 核实用例环境：按 `VerifySpec` 装配一个只读核实用例 `CoreVerify`（工具面、放行名单、围栏）；调用方给沙箱，本文件不取事实 |
| `src/units/business/agentloop/domain/ports.rs` | 装配一席的端口参数对象 `SessionPorts`（llm / 日志 / 提示词册 / 工具总表 / 工具执行 / 虚拟机 / 常驻服务 / 工作区 / 压缩通道 / 会话形状声明 `SessionDecls`）；与装配机同单元，否则环 2 内成环 |
| `src/units/business/agentloop/domain/reply.rs` | 消息回灌：一次模型回复与工具结果 → 发给模型的消息序列（实时与重建的唯一来源） |
| `src/units/business/agentloop/engine.rs` | 引擎门面：声明 `core_op` / `decls` / `driver` / `io` / `round` / `tools` 六个子模块 |
| `src/units/business/agentloop/engine/driver.rs` | 单回合驱动：在一条 `AgentSession` 上跑发言 / 派发 / 讨论席回合 / 继续 / 压缩（五个 `&mut self` 方法，取消与短暂事件出口按形参传） |
| `src/units/business/agentloop/engine/core_op.rs` | 核心操作回路：在工具面上跑一次核心操作（面 = 本条动作 + 读面，不再跨段合并；取载荷、走只读核实、把事实推给调用方），取消与分片的包装只在这里 |
| `src/units/business/agentloop/engine/decls.rs` | 工具声明面的值类型：`WireTools` / `Decls` / `ToolDecls`（`round` 与 `tools` 共用一份） |
| `src/units/business/agentloop/engine/io.rs` | 回合 IO 与结论：`TurnIo`（调用参数 / 裁决队 / 提问端口）与 `MemberTurn` / `CompactedTurn` |
| `src/units/business/agentloop/engine/round.rs` | 轮循环与行构造：请求装配 `assemble`、`converse_with`、`build_round_lines`（唯一构造函数）、轮 / 回复词汇 |
| `src/units/business/agentloop/engine/tools.rs` | 成员回合的工具循环：声明面 → 放行（在场看面、准不准问 `Policy`）→ 并发调度 → 执行 → 回填；shell 与模块命令都经 `Sandbox::precheck` / `run`，引擎不按执行环境分叉；版本化动作全部经工作台策略给的 `BenchDriver`（门禁 / 写前拦 / 写后存档 / 批末采纳 / 过期读取），引擎不认识版本库 |
