# 单元地图 · packages

> 本文是 **packages** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/packages/api.rs` | 入站面：`Packages` 用例 trait（包库 / 依赖文件夹）、`PackagePlan` / `PlannedPackage` / `Problem` 的重导出 |
| `src/capabilities/packages/service.rs` | 用例与端口持有者：持 `PackageSource`（R12），交出包库快照与目录 |
| `src/capabilities/packages/ports.rs` | 出站端口：`PackageSource`（扫描 runtimes/ 包清单） |
| `src/capabilities/packages/domain/manifest.rs` | 包契约（package.yaml）校验、去重、系统路径冲突预检与平台分包选择（纯逻辑） |
| `src/capabilities/packages/domain/plan.rs` | 选型诊断、按能力选包、装载顺序、构建期包计划与不可用能力汇总（纯逻辑） |
| `src/capabilities/packages/detail/fs_packages.rs` | `PackageSource` 的磁盘实现：扫描 runtimes/ |
