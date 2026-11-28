# secrets（隐秘字段）

> 定位：模块声明的隐秘信息（如 MCP server 的令牌）的值存储与按模块解析——值只落 `data/.home/`，只给起进程的一侧注入。
> 效力：规范｜核对：2026-11-28｜依据：`src/units/business/catalog/secrets/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：模块隐秘字段的声明视图（配置了没有）、置值 / 清值与落盘、按模块解析注入项（env 名 → 值）、按已知值脱敏。
- 不管：模块声明的解析（在 `modules`）。
- 不管：通道密钥（归 `registry` 的 `providers.yaml`）。
- 不管：实际注入子进程（由工具 / 常驻服务在起进程时按 `resolve` 消费）。

## 入站契约与状态归属

- 入站面：`api`（`SecretOps` + `SecretView` + 空实现 `NoSecrets`）。
- 出站端口：`SecretStore` **只由 `service.rs` 持有**（R12）。
- 状态归属：值缓存（键 = 模块 / 字段）+ 存储端口；文件实现在 `src/units/business/catalog/secrets/detail/yaml_secrets.rs`（`data/.home/` 的 `secrets.yaml`，unix 下 0600）。

## 依赖图位置（由源码的 `::api` 引用推导）
- 所属单元：`catalog`（环 2）；本页的子业务同单元的 `modules` / `packages` / `registry` 子业务直接调用，不经 `api`。
- 本子业务经 `::api` 用到：`catalog`（`modules`）、`kernel`
- 谁在用本子业务：`entry`、`presentation`（`Faces.secrets`）、`residents`（起服务时解析注入项）、`runtime`、`turn_state`

## 改动本单元时必须同步

- 值存储的字段或文件形态 → `REGISTRY_SPEC.md` 的密钥边界；端口替身 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。
- 模块的 `secrets:` 声明契约见 [MODULE_SPEC.md](../../MODULE_SPEC.md)。
- 消费点：
  - `declared`：CLI 的 `secret` 命令（只列标识与配置了没有）。
  - `resolve`：起进程的三条路径——agent 的模块工具进程（`runtime` 装配工具面时按模块解析一次）、常驻服务（`residents` 起服务时）、人直接跑的模块工具（`runtime` 的动作执行体）；只经环境变量注入，不进命令行。
  - `redact`：常驻服务的回执出口。
  - `set` / `clear`：动作面 `set_secret` / `clear_secret`（只给 user，值不回显）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
