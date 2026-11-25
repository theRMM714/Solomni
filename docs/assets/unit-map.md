# 单元地图 · assets

> 定位：assets（随包只读内容）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/assets/api.rs`

> 本文是 **assets** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/assets.rs` | 单元根：只声明 `api` / `domain` / `ports` / `service` / `detail` |
| `src/units/assets/api.rs` | 入站面：`Assets`（读文档 / 枚举提示词册）、`Doc`、`PromptDoc`、`paths()` 落点派生 |
| `src/units/assets/ports.rs` | 出站端口 `AssetFiles`（按路径读全文、递归收 `.yaml`） |
| `src/units/assets/service.rs` | 用例：把 `Doc` 翻成端口调用；提示词册排序与读失败文案 |
| `src/units/assets/domain.rs` | 纯定义入口：只声明 `doc` 与 `paths` |
| `src/units/assets/domain/paths.rs` | 落点派生：`AssetPaths`（`prompts` / `systools` / `locales`） |
| `src/units/assets/domain/doc.rs` | 固定文档清单：`Doc` 的 `file_name` / `label` |
| `src/units/assets/detail.rs` | 机制入口：只声明 `fs_assets` 并重导出 `FsAssetFs` |
| `src/units/assets/detail/fs_assets.rs` | 真实文件适配器：`read_to_string` 与递归目录收集（不跟符号链接） |
