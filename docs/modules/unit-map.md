# 单元地图 · modules

> 定位：modules（模块公地）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/catalog/modules/api.rs`

> 本文是 **modules** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/catalog/modules/api.rs` | 入站面：模块契约类型、`ModulesOps`（只读面）与 `Modules`（用例面），以及入口判定（`EntryProblem` / `MissingEntry` / `missing_entries_for` / `runs_on_target`） |
| `src/units/business/catalog/modules/service.rs` | 用例与端口持有者：`ModuleSource` 只在这里（R12） |
| `src/units/business/catalog/modules/ports.rs` | 出站端口：`ModuleSource`（扫描 `modules/`） |
| `src/units/business/catalog/modules/domain/module.rs` | `module.yaml` 契约、参数类型、校验与清单（`Roster` / `listing`） |
| `src/units/business/catalog/modules/domain/entry.rs` | 入口判定（纯逻辑）：按平台挑模块声明的入口候选 `missing_entries_for`，以及一个文件能不能在这个目标系统上跑 `runs_on_target`（ELF / PE / 脚本 / Mach-O 的文件头判定） |
| `src/units/business/catalog/modules/domain/state.rs` | 启停覆盖值与文件形态（`data/.home/modules.yaml` 的 serde 形态） |
| `src/units/business/catalog/modules/detail/fs_modules.rs` | 适配器：扫描目录、解析清单；私有区是否可用问 `content`（`ensure_userdata`），自己不在源码树里建目录 |
| `src/units/business/catalog/modules/detail/yaml_module_state.rs` | 适配器：读写 `data/.home/modules.yaml` |
