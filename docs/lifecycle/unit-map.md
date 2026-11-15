# 单元地图 · lifecycle

> 定位：lifecycle（会话历史生命周期）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/lifecycle/api.rs`

> 本文是 **lifecycle** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**会话历史读侧与删除收尾的唯一入口**：从 `session` 的存储端口读、按 `meta.parent` 派生树序与子树，
> 删除时只做收尾段（停 guest → 删目录 → 后台撤权），不持落盘状态，删前的内存动作留在 `conductor`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/lifecycle.rs` | 能力入口与公开面 |
| `src/capabilities/lifecycle/api.rs` | 入站契约：列表 / 打开 / 树序 / 子树与删除收尾类型的重导出 |
| `src/capabilities/lifecycle/domain/history.rs` | 纯逻辑：树序排列与子树收集，不碰盘、不碰日志 |
| `src/capabilities/lifecycle/service/history.rs` | 读侧：树序列表（失败记日志返回空表）与 rewind 截断后的只读回放 |
| `src/capabilities/lifecycle/service/delete.rs` | 收尾段：先停 guest、先子后父删目录、后台按租约撤围栏 |
