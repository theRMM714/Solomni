# 单元地图 · turn

> 定位：turn（会话在世状态）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/turn/api.rs`

> 本文是 **turn** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**会话在世状态的唯一持有者**：五张内存表（在世表 / 运行态 / 占名表 / 裁决队句柄 / shell 状态槽）都装在这里，
> 只经访问器读写、不落盘；落盘与转录归 `session`，命令队列与派发闸归 `conductor`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/turn.rs` | 能力入口与公开面 |
| `src/capabilities/turn/api.rs` | 入站契约：`TurnState` 与 `Session` 的重导出 |
| `src/capabilities/turn/service.rs` | 服务层入口：状态容器模块 |
| `src/capabilities/turn/service/state.rs` | 状态容器：五张内存表与全部访问器；`Session` 枚举本体 |
