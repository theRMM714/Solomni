# 单元地图 · transcript

> 定位：transcript（转录）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/transcript/api.rs`

> 本文是 **transcript** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/transcript/api.rs` | 入站面：重导出事件、转录行、裁决卡与上下文用量及其构造函数（别的单元与呈现层只准用这里） |
| `src/units/business/transcript/domain.rs` | 纯定义的入口：声明 `events` 概念子模块 |
| `src/units/business/transcript/domain/events.rs` | 线格式词汇：`SessionEvent` 与机制事件通道 `Push`、`LineView` / `ToolCallView`、裁决卡信封（`card_of` / `waiter_of` / `event_of` / `json_of` / `note_of`：`kernel::Ask` → 卡 / 等待方 / 事件）、`ContextUsage` 与其构造与判定 |
