# 单元地图 · exec_model

> 定位：exec_model（执行与作业模型）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/pure/exec_model/api.rs`

> 本文是 **exec_model** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/pure/exec_model/api.rs` | 入站面：重导出 `Side` / `TargetSpec` / `JobState`（其它单元与呈现层只准用这里） |
| `src/units/pure/exec_model/domain.rs` | 纯定义的入口：声明 `exec_selection` 与 `job` 两个概念子模块 |
| `src/units/pure/exec_model/domain/exec_selection.rs` | `Side`（`host` / `guest` 哪一侧）与 `TargetSpec`（侧 + 客机名 + 能力定版；由执行环境事实派生，不落盘） |
| `src/units/pure/exec_model/domain/job.rs` | `JobState`：长任务进度快照（`running` / `seq` / `lines` / `error`） |
