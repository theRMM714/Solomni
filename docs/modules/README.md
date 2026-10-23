# modules（模块公地）

> 模块的发现、`module.yaml` 契约与启停状态——**所有需要模块的人的上游**。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：扫描 `modules/`、`module.yaml` 契约与校验、模块清单（`Roster`）、清单文本（`listing`）、模块与服务的启停状态（有效值 = 用户覆盖 ?? 作者默认）。

**不管**：工具面与参数渲染（`tools` / `session` 消费模块声明）、常驻服务的运行态与生命周期（`residents`）、隐秘字段的**值**（`secrets`）、运行包库与选包（`packages`）、执行计划（`vm`）、身份块渲染（`session`）、名单/登记/装配（`slate` / `registry` / `conductor`）。

## 二、入站契约与状态归属

`api::ModulesOps`（只读面：`roster` / `module_enabled` / `service_enabled`）+ `api::Modules`（用例面：再加 `set_module_enabled` / `set_service_enabled`）。出站端口 `ModuleSource` 与 `ModuleStateStore` **只由 `service.rs` 持有**（R12）。
启停语义：有效值 = 用户覆盖（`.home/modules.yaml`）?? 作者默认（`module.yaml` 顶层 `enabled` 与 `services.<名>.enabled`）；**模块停用 ⇒ 它的服务一律有效地停用**。
契约类型（`Module` / `ModuleManifest` / `Roster` / `ToolDecl` / `ServiceDecl` / `SecretDecl` / `Param` / `ParamType`）由本能力拥有，其它能力只经 `api` 引用。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`（能力名文法 / 机制）、`prompt`（清单文本）
- 谁在用我：`collab`、`conductor`、`registry`、`residents`、`secrets`、`session`、`slate`、`tools`、`vm`

## 四、改动本单元时必须同步

- `module.yaml` 契约 → [MODULE_SPEC.md](../../MODULE_SPEC.md)；`Param` / `ParamType` 被 `tools` 的 `ToolSchema` 消费。
- 清单扫描 / 校验用例：`src/tests/modules.rs`；扫描适配器的契约在 `src/tests/detail.rs`。
- 端口矩阵（`ModuleSource`）→ [docs/testing/doubles.md](../testing/doubles.md) 三。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
