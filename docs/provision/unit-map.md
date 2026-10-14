# 单元地图 · provision

> 本文是 **provision** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 依赖补齐能力：清单 / 台账解析、本机状态、下载缓存、解包落盘与后台作业。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/provision/mod.rs` | 能力入口：模块与公开面（`ProvisionService`） |
| `src/capabilities/provision/api.rs` | 入站契约：`Provision` trait + `DepStatus` / `JobState` DTO + `NoProvision` |
| `src/capabilities/provision/service.rs` | 实现：清单 / 台账解析、本机状态、下载缓存、解包落盘、作业与取消 |