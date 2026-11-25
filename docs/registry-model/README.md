# registry_model（登记处读模型）

> 定位：纯领域——agent、供应商、模型的存档形态、视图与名字/校验规则的唯一家（环 1）。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/pure/registry_model/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：agent 的存档形态 `Agent` 与入档草稿 `AgentDraft`、会话名单的唯一形状 `AgentMeta`（R15）、视图 `AgentView` 与名单聚合 `Agents`（`BTreeMap<String, Agent>`）、拟名条目 `RosterPick`；名字规则 `validate_name` / `check_reserved`；名录渲染与点名 `agent_meta` / `listing` / `views` / `pick`。
- 管：供应商与模型的值形态 `Provider` / `ModelEntry` / `ModelDraft`、工具调用形态 `ToolMode`、思考档位 `ThinkingLevel` / `ThinkingCatalog` / `ThinkingSpec`、缓存预算 `StorageSettings`、三张视图 `ProviderView` / `ModelView` / `ModelThinkingView` 与思考目录校验 `validate_thinking_catalog`。
- 不管：名单落地与唯一实例命名（`src/units/catalog/registry/domain/picks.rs` 的 `resolve_picks` / `unique_instance_name` / `model_listing` 要读 modules 的 `Roster`）；设置状态与通道解析（`src/units/catalog/registry/domain/providers.rs` 的 `AppSettings` / `SandboxSettings` / `Settings` 与 `resolve`）；四份 yaml 的读写（`src/units/catalog/registry/detail/yaml_settings.rs`）；登记处的写用例与端口（`src/units/catalog/registry/service.rs` / `ports.rs`）。
- 不管：不碰 IO、不持端口、不持有状态——环 1 的判据见 [layers.md](../architecture/layers.md) 的 H2。

## 入站契约与状态归属

- 入站面：`api`——重导出上述形状与函数。
- 出站端口：无（纯领域，不自持端口）。
- 状态归属：不持有状态；agent 名单与模型清单由登记处（环 2）持有，会话只持有 `AgentMeta` 快照。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：无（环 1，只依赖标准库与 serde）。
- 谁在用我：`catalog`、`slate`、`turn`、`env`、`runtime`、`work`、`lifecycle`、`presentation` 与测试。

## 一、为什么是独立单元

- `AgentMeta` 是全仓引用面最大的共享形状之一（44 个文件），会话、协作、回合、运行时与两个前端都读它；它原先长在 `catalog`（环 2）的 registry 子业务里，等于「为取值而依赖整个登记处」。
- 拆出来以后：agent 的形状与名字规则有唯一家，任何单元与适配层都只经 `units::pure::registry_model::api` 取；登记处只负责读写与用例，不再替所有人保管形状（R15）。
- 为什么是环 1 而不是留在环 2：这些值不碰 IO、不持端口、可穷举测试（名字校验与保留名是纯函数）；按 H2 它们就该在没有 IO 的层。
- 供应商与模型的值形态已随 P4b 迁入本单元（`src/units/pure/registry_model/domain/models.rs`），`ToolMode` 一并归环 1（它描述「这个模型怎么调」，是登记处的事实，不是 llm 协议的私有词汇）；仍留在 `catalog` 的是**设置状态与通道解析**：`Settings::resolve` 要构造 `llm` 的 `Channel`，`SandboxSettings.scope` 仍是 `workspace` 的 `SharedScope`——两者都得等 workspace 的共享区值模型下沉（H3）后再搬。

## 改动本单元时必须同步

- 改 `AgentMeta` 字段（R15 的会话名单形状）→ `docs/session/session-model.md`、会话 `meta` 读写、`src/tests/session/`。
- 改 `Agent` / `AgentDraft` 字段或名字规则 → `REGISTRY_SPEC.md`（字段与拒收规则的唯一权威）、`src/tests/registry.rs`、`docs/registry/README.md`。
- 改 `ModelEntry` / `ThinkingSpec` / `ToolMode` 字段或视图形状 → `REGISTRY_SPEC.md`、`src/tests/registry.rs`、`docs/registry/README.md` 与 `src/presentation/web/assets/app/`（设置页表单）。
- 改 `RosterPick` 的变体 → 拟名单的提示词与解析（`src/units/conversation/slate/`）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
