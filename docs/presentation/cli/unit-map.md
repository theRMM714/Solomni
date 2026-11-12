# 单元地图 · presentation/cli

> 定位：presentation/cli（终端呈现）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-06｜依据：`src/presentation/cli/mod.rs`

> 本文是 **presentation/cli** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/presentation/cli/mod.rs` | 终端转录中心入口：argv 解析 → 选命令或会话流；不做业务判断、不持状态 |
| `src/presentation/cli/commands.rs` | 一次性命令：`module` / `diagnose` / `secret` / `resident` / `work` / `rewind` |
| `src/presentation/cli/view.rs` | 渲染原语：花名册、菜单、事件流、模型与供应商标签 |
| `src/presentation/cli/gate.rs` | 交互回合：跟读事件流、裁决卡问答、等安静与动作回显 |
| `src/presentation/cli/flows.rs` | 会话流：单 agent / 代理 / 协作 / 供应商 / 模型 / 核心设置 |
