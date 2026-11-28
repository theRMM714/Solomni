# 单元地图 · vmstore

> 定位：vmstore（虚拟机盘资源）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-28｜依据：`src/units/resource/vmstore/api.rs`

> 本文是 **vmstore** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/resource/vmstore.rs` | 单元根：只声明 api / domain |
| `src/units/resource/vmstore/api.rs` | 入站面：`paths(root) -> Paths`、`resolve_guest(root, guest)`（虚拟根名 → 那一格目录，名字是单纯一段）与 `Paths` 再导出 |
| `src/units/resource/vmstore/domain.rs` | 纯逻辑入口：声明 `paths` |
| `src/units/resource/vmstore/domain/paths.rs` | `.vm` 全部落点的拼法与路径段安全化（含两个布局单测） |
