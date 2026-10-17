# 单元地图 · registry

> 本文是 **registry** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/registry/api.rs` | 入站能力面：登记处 DTO 的重导出、`Registry` 能力面与呈现层的队列面 `RegistryOps`（内部线程 + 命令队列） |
| `src/capabilities/registry/service.rs` | 登记处的状态与用例：持四份 yaml 的内存形态（`Settings`），改完即经 `SettingsStore` 落盘 |
| `src/capabilities/registry/ports.rs` | 出站端口：四份 yaml 的持久化 `SettingsStore`（load / save） |
| `src/capabilities/registry/domain/providers.rs` | 供应商与模型的内存形态与通道解析：`Provider`（端点 + 密钥）、`ModelEntry`（模型串 / 上下文 / 工具形态）与 `Settings` / `AppSettings` |
| `src/capabilities/registry/domain/agents.rs` | agent 登记处的内存形态与纯逻辑：具名 agent（模块 + 模型 + 说明）的校验、唯一实例名、名单渲染与挑选 |
| `src/capabilities/registry/detail/yaml_settings.rs` | `SettingsStore` 的磁盘实现：providers / models / settings / agents 四份 yaml 的读写（密钥只落 `providers.yaml`，unix 下 0600） |
