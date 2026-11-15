# 单元地图 · turn

> 定位：turn（回合状态）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/turn/api.rs`

> 本文是 **turn** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**回合状态的唯一持有者**：会话在世表、运行态与创建占位、每会话的裁决队与 shell 活进程槽都只有这里增删，
> 别的业务经 `api` 取用；会话本体与生成驱动不在本单元内。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/turn.rs` | 能力入口与公开面 |
| `src/capabilities/turn/api.rs` | 入站契约：`Session`（两种在世形态）与 `TurnState`（访问器）的重导出 |
| `src/capabilities/turn/service.rs` | 服务分区声明 |
| `src/capabilities/turn/service/state.rs` | 状态本体与访问器：在世表、运行态、创建占位、裁决队表、每会话 shell 槽 |
