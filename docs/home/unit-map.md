# 单元地图 · home

> 定位：home（登记处资源）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-28｜依据：`src/units/resource/home/api.rs`

> 本文是 **home** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/resource/home.rs` | 单元根：只声明子业务（`domain` / `detail`）与 api / ports / service |
| `src/units/resource/home/api.rs` | 入站面：`Home`（读 / 整份写）与 `Doc` 重导出；文本原样出入 |
| `src/units/resource/home/ports.rs` | 出站端口 `HomeFiles`（按文件名读写，`private` 决定权限） |
| `src/units/resource/home/service.rs` | 用例：把 `Doc` 映射成文件名与私密标记，交给端口 |
| `src/units/resource/home/domain.rs` | 纯定义入口：只声明 `doc` |
| `src/units/resource/home/domain/doc.rs` | `Doc` 的取值、文件名、是否私密 |
| `src/units/resource/home/detail.rs` | 机制入口：只声明 `fs_home` 并重导出 `FsHome` |
| `src/units/resource/home/detail/fs_home.rs` | 真实文件适配器：读（NotFound→None）、原子写、私密权限 |
