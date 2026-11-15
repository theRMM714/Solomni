# 单元地图 · turn

> 定位：turn（会话本体 + 会话装配构件）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/turn/api.rs`

> 本文是 **turn** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**会话本体的唯一持有者**：五张内存表（在世表 / 运行态 / 占名表 / 裁决队句柄 / shell 状态槽）与各能力面都装在这里，
> 取放协议、装配、从落盘重建、树序与身份、裁决队作废也都实现在这里；只经访问器读写、不落盘——落盘归 `session` 的 `History`，
> 写点由调用方 `conductor` 执行。命令队列、跨会话编排与生成驱动归 `conductor`。
> 它也是**一席会话的装配构件**：调用方把事实交来，它装出工具面与会话对象，纯规则在 `src/capabilities/turn/domain/`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/turn.rs` | 能力入口与公开面 |
| `src/capabilities/turn/api.rs` | 入站契约：`Turn` / `TurnState` / `Session`、装配构件（`MemberSpec` / `SingleSpec` / `member_tools` / `single_session`）、树序与身份、装配要的事实、执行计划、代理会话私有目录名、转录回放与三条纯规则的重导出 |
| `src/capabilities/turn/domain.rs` | domain 分区入口：纯规则模块 |
| `src/capabilities/turn/domain/assemble.rs` | 纯装配规则：工具面按权限裁剪（`face_tools`）、已就绪服务操作归并（`service_ops_of`）、操作写进模块工具表并给出提示词行（`serviced_tools`）；无端口、无 IO |
| `src/capabilities/turn/domain/session.rs` | 纯会话规则：多个 agent 的模块保序去重（`flat_modules`）、流水里是否已有内容（`session_started`）；无端口、无 IO |
| `src/capabilities/turn/service.rs` | 服务层入口：会话本体、取放协议、树序、事实、计划、重建、代理装配与装配构件九个模块 |
| `src/capabilities/turn/service/session.rs` | 会话本体 `Turn`：字段（五张表 + 各能力面 + 产品根 + 基础工具链根）、构造与后置注入、`session_deps` / `compact_factory`、`running_refusal` 文案，以及 `TurnState` 的同名访问器转发 |
| `src/capabilities/turn/service/state.rs` | 状态容器：五张内存表与全部访问器；`Session` 枚举本体 |
| `src/capabilities/turn/service/protocol.rs` | 取放协议与会话派生：`take_single` / `take_collab` / `take_collab_raw` / `put_single` / `put_single_recorded` / `put_collab` / `abort_running`、裁决队作废（`void_gates_of` / `void_gates_in_subtree`）、节点提示（`node_note` / `mark_node_done`）、`spawn_agent_session` / `spawn_sub_session`、给代理父会话的提醒（`notify_proxy_of_child`，只返回事件不落盘） |
| `src/capabilities/turn/service/assemble.rs` | 一席装配构件与会话启用：`MemberSpec` / `SingleSpec`、`member_tools`（工具面 + 围栏 + 按放行名单现取的工作工具处理器）、`single_session`（单 agent 会话对象）、`ensure_session`（不在表里按转录重建再装回） |
| `src/capabilities/turn/service/tree.rs` | 树序与身份：`subtree_of`、`work_root_of` / `work_root`（成环拒绝）、`session_kind_of` / `role_of`、`tree_meta` / `tree_meta_with`、`workspace_binding_of`、派发闸（`dispatch_gate` / `dispatch_target`）、历史列表 |
| `src/capabilities/turn/service/facts.rs` | 装配要的事实：guest 作用域与租约、配置归属、沙箱集合、权限链与权限面、共享区授权、上下文预算与刷新 |
| `src/capabilities/turn/service/plan.rs` | 执行计划与只读根：`guest_image_of` / `guest_platform_of` / `guest_runtimes_of` / `guest_os_of` / `path_codec_of` / `library_for`、`host_plan_for` / `plan_for_spec` / `plan_for_meta_with` / `unavailable_modules`、网络开关与主机只读根 |
| `src/capabilities/turn/service/rebuild.rs` | 从落盘重建：`rebuild_session`（single / collab / proxy 三形态）、`replay_dialogue`（发送视图与压缩点）、`replay_reads`（读事实） |
| `src/capabilities/turn/service/proxy.rs` | 代理会话装配：按父会话 meta 装出代理会话（私有工作目录、核心通道、角色脸），`PROXY_PRIVATE_DIR` |
