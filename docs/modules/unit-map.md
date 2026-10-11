# 单元地图 · modules

> 本文是 **modules** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/modules/api.rs` | 入站能力面：模块契约类型、`ModulesOps`（只读面）与 `Modules`（用例面） |
| `src/capabilities/modules/service.rs` | 用例与端口持有者：`ModuleSource` 只在这里（R12） |
| `src/capabilities/modules/ports.rs` | 出站端口：`ModuleSource`（扫描 `modules/`） |
| `src/capabilities/modules/domain/module.rs` | `module.yaml` 契约、参数类型、校验与清单（`Roster` / `listing`） |
| `src/capabilities/modules/domain/state.rs` | 启停覆盖值与文件形态（`.home/modules.yaml` 的 serde 形态） |
| `src/capabilities/modules/detail/fs_modules.rs` | 适配器：扫描目录、解析清单、确保 `userdata/` 存在 |
| `src/capabilities/modules/detail/yaml_module_state.rs` | 适配器：读写 `.home/modules.yaml` |
