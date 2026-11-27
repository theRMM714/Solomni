# 单元地图 · registry_model

> 定位：registry_model（登记处读模型）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/pure/registry_model/api.rs`

> 本文是 **registry_model** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/pure/registry_model/api.rs` | 入站面：重导出 agent / 供应商 / 模型的存档形态、视图、名字规则与思考目录校验（其它单元与呈现层只准用这里） |
| `src/units/pure/registry_model/domain.rs` | 纯定义的入口：声明 `agents`、`models` 概念子模块 |
| `src/units/pure/registry_model/domain/agents.rs` | agent 的存档形态与纯逻辑：`Agent` / `AgentDraft` / `AgentMeta` / `AgentView`、拟名条目 `RosterPick`、名字校验与保留名、名录渲染与点名 |
| `src/units/pure/registry_model/domain/params.rs` | 参数声明的形状 `Param` / `ParamType`：模块工具、系统工具、命令、会话类型与沙箱策略共用（从 `catalog::modules` 下沉，环 1 是它的唯一家） |
| `src/units/pure/registry_model/domain/models.rs` | 供应商与模型的值形态：`Provider` / `ModelEntry` / `ModelDraft`、工具调用形态 `ToolMode`、思考档位 `ThinkingLevel` / `ThinkingCatalog` / `ThinkingSpec`、`StorageSettings`、三张视图与思考目录校验 `validate_thinking_catalog` |
