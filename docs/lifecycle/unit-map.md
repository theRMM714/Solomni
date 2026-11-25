# 单元地图 · lifecycle

> 定位：lifecycle（会话历史生命周期）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/lifecycle/api.rs`

> 本文是 **lifecycle** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**会话历史生命周期的唯一落点**：回档三模式、历史列表与只读打开、删除的报账与收尾都只在这里实现。
> 它**不持会话状态、不持端口**：会话本体与运行态归 `turn`，登记处与探针按次经 `RebuildDeps` 传入。

| 文件 | 职责 |
| --- | --- |
| `src/units/lifecycle.rs` | 单元入口与公开面 |
| `src/units/lifecycle/api.rs` | 入站面：`RewindTarget`（留档 / 删除按行 id，还原按标记 id）、`RewindOutcome`（主视图 + 子会话视图）、`RebuildDeps`（登记处与探针按次传入）；删除收尾对象与撤权回调经这里转出 |
| `src/units/lifecycle/service.rs` | 方法族入口：`Lifecycle` 的六个入站面与构造（组合根专用，不新建实现） |
| `src/units/lifecycle/service/rewind.rs` | 回档三模式：留档（标记 + 折叠）、删除（收窗 + 截断）、还原（删标记 + 重放）；`restore_session` 重建并放回、`next_mark_id` 续号、`keep_by_agent` 按 agent 收窗 |
| `src/units/lifecycle/service/history.rs` | 历史列出 / 只读打开 / 删除收尾：`FenceRelease` 后台撤权、`SessionDelete::run` 先停 guest 再删目录、`delete_bookkeeping` 的报账与报错 |
