# 单元地图 · sessions

> 定位：sessions（会话盘资源）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/sessions/api.rs`

> 本文是 **sessions** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/sessions.rs` | 单元根：只声明 api / ports / service / detail |
| `src/units/sessions/api.rs` | 入站面：`Sessions`（会话根 + 会话目录存在性） |
| `src/units/sessions/ports.rs` | 出站端口 `SessionDirs`（目录存在性判定） |
| `src/units/sessions/service.rs` | 用例：会话根与会话名的拼接，把事实问题交给端口 |
| `src/units/sessions/detail.rs` | 机制入口：只声明 `fs_dirs` 并重导出 `FsSessionDirs` |
| `src/units/sessions/detail/fs_dirs.rs` | 真实目录适配器：`is_dir` 这一步宿主判定 |
