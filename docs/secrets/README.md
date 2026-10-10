# secrets（隐秘字段）

> 模块声明的隐秘信息（如 MCP server 的令牌）的值存储与按模块解析：值只落 `.home/`，只给起进程的一侧注入。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：模块隐秘字段的声明视图（配置了没有）、置值 / 清值与落盘、按模块解析注入项（env 名 → 值）、按已知值脱敏。

**不管**：模块声明的解析（在 `workspace`）；通道密钥（归 `registry` 的 `providers.yaml`）；实际注入子进程（由工具 / 常驻服务在起进程时按 `resolve` 消费）。

## 二、入站契约与状态归属

`api`（`SecretOps` + `SecretView` + 空实现 `NoSecrets`）；出站端口 `SecretStore` **只由 `service.rs` 持有**（R12）。
状态在 `service.rs`：值缓存（键 = 模块/字段）+ 存储端口；文件实现在 `detail/yaml_secrets.rs`（`.home/secrets.yaml`，unix 下 0600）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`workspace`（取模块清单事实）
- 谁在用我：`conductor`（`Ops` 字段；Phase 1 由组合根注入）

## 四、改动本单元时必须同步

- 值存储的字段或文件形态 → [REGISTRY_SPEC.md](../../REGISTRY_SPEC.md) 的密钥边界；端口替身 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。
- 模块的 `secrets:` 声明契约见 [MODULE_SPEC.md](../../MODULE_SPEC.md)。
- 消费点：`declared` 由 CLI 的 `secret` 命令、`resolve`/`redact` 由 `residents`（起服务注入 env 与回执脱敏）消费；`set`/`clear` 随设置面接入，在此之前只被契约测试驱动（`#![allow(dead_code)]` 如实标注）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
