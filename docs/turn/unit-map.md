# 单元地图 · turn

> 定位：turn（回合状态）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/turn/api.rs`

> 本文是 **turn** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**回合状态的唯一持有者**：会话在世表、运行态与创建占位、每会话的裁决队与 shell 活进程槽都只有这里增删，
> 别的业务经 `api` 取用。会话本体（`Turn`）与一席的装配材料也在这里收拢：准入与落盘、按需重建、派发闸、三形态重建与代理装配都归它，
> 但**不驱动生成**——生成循环与命令队列在 `conductor`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/turn.rs` | 能力入口与公开面 |
| `src/capabilities/turn/api.rs` | 入站契约：`Session`（两种在世形态）、`TurnState`（访问器）、`Turn`（会话本体）、装配材料（`SessionDeps` / `MemberSpec` / `SingleSpec` / `member_tools` / `single_session`）、转录回放（`replay_dialogue`）与代理会话常量（`PROXY_PRIVATE_DIR` / `SPEAKER`）的重导出 |
| `src/capabilities/turn/service.rs` | 服务分区声明 |
| `src/capabilities/turn/service/state.rs` | 状态本体与访问器：在世表、运行态、创建占位、裁决队表、每会话 shell 槽 |
| `src/capabilities/turn/service/session.rs` | 会话本体 `Turn`：持状态、装配要的能力面（`workspace` / `vm` / `residents` / `secrets` / `tools` / `systools` / `prompt` / `llm` / `log` / `compact` / `base_roots`）与重建要的四处事实（`root` / `history` / `modules` / `packages`），转发全部状态访问器，给 `running_refusal` 的唯一实现 |
| `src/capabilities/turn/service/admit.rs` | 会话准入：`admit`（落盘 + 进在世表）、`ensure_session`（不在世就重建再放入）、`dispatch_gate`（终态会话拒绝派发）、`resume_subtree`（把停掉的子树拉回运行态） |
| `src/capabilities/turn/service/facts.rs` | 一席事实的取用：沙箱清单、权限面、外部工作区授权粒度、工具面（`member_tools`）与上下文预算；只经各能力 `api` 取，不持端口 |
| `src/capabilities/turn/service/rebuild.rs` | 从落盘重建会话：外部工作区绑定复检（`workspace_binding_of`）、三形态重建（`rebuild_session`）、转录回放（`replay_reads` / `replay_dialogue`）与代理会话装配（`build_proxy`） |
| `src/capabilities/turn/service/assemble.rs` | 一席的装配：`SessionDeps`（能力面）、`MemberSpec` / `SingleSpec`（取来的本席事实）、`member_tools`（工具面）与 `single_session`（单 agent 会话对象） |
| `src/capabilities/turn/domain.rs` | 纯派生分区声明 |
| `src/capabilities/turn/domain/assemble.rs` | 装配的纯函数：`face_tools`（发放名单过滤）、`service_ops_of`（就绪服务 → 工具）、`serviced_tools`（服务工具与提示行注入模块表） |
