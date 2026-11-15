# 单元地图 · turn

> 定位：turn（会话在世状态）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/turn/api.rs`

> 本文是 **turn** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**会话在世状态的唯一持有者**：五张内存表（在世表 / 运行态 / 占名表 / 裁决队句柄 / shell 状态槽）都装在这里，
> 只经访问器读写、不落盘；落盘与转录归 `session`，命令队列与派发闸归 `conductor`。
> 它也是**一席会话的装配构件**：调用方把事实交来，它装出工具面与会话对象，纯规则在 `src/capabilities/turn/domain/assemble.rs`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/turn.rs` | 能力入口与公开面 |
| `src/capabilities/turn/api.rs` | 入站契约：`TurnState` / `Session` 与装配构件（`SessionDeps` / `MemberSpec` / `SingleSpec` / `member_tools` / `single_session` / 三条纯规则）的重导出 |
| `src/capabilities/turn/domain.rs` | domain 分区入口：纯装配规则模块 |
| `src/capabilities/turn/domain/assemble.rs` | 纯装配规则：工具面按权限裁剪（`face_tools`）、已就绪服务操作归并（`service_ops_of`）、操作写进模块工具表并给出提示词行（`serviced_tools`）；无端口、无 IO |
| `src/capabilities/turn/service.rs` | 服务层入口：状态容器与装配构件两个模块 |
| `src/capabilities/turn/service/assemble.rs` | 一席装配构件：`SessionDeps`（十个能力面）、`MemberSpec` / `SingleSpec`、`member_tools`（工具面 + 围栏 + 处理器）、`single_session`（单 agent 会话对象） |
| `src/capabilities/turn/service/state.rs` | 状态容器：五张内存表与全部访问器；`Session` 枚举本体 |
