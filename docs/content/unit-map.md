# 单元地图 · content

> 定位：content（内容盘资源）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/content/api.rs`

> 本文是 **content** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/content.rs` | 单元根：只声明子业务（`domain` / `detail`）与 api / ports / service |
| `src/units/content/api.rs` | 入站面：`paths()` 落点派生、`Content`（私有区 / 运行包根）、`ContentInstall`（安装/卸载形状，本期缺口） |
| `src/units/content/ports.rs` | 出站端口 `ContentFs`（目录的确保存在） |
| `src/units/content/service.rs` | 用例：持端口，把模块 id 变成私有区并确保它存在；安装/卸载如实报缺口 |
| `src/units/content/domain.rs` | 纯定义入口：只声明 `paths` |
| `src/units/content/domain/paths.rs` | 落点派生：`ContentPaths` / `ModuleArea`、模块 id 的路径段安全化 |
| `src/units/content/detail.rs` | 机制入口：只声明 `fs_content` 并重导出 `FsContentFs` |
| `src/units/content/detail/fs_content.rs` | 真实文件适配器：`create_dir_all` 的幂等确保存在 |
