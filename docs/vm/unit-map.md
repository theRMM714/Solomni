# 单元地图 · vm

> 本文是 **vm** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/vm/api.rs` | 入站面：`Vm` 用例 trait（运行包库 / 包库目录）与档位、计划、诊断、就绪检测的重导出 |
| `src/capabilities/vm/service.rs` | 用例与端口持有者：持 `PackageSource`（R12），实现 `Vm` |
| `src/capabilities/vm/ports.rs` | 出站端口 `PackageSource`（扫描 runtimes/ 里的包清单） |
| `src/capabilities/vm/domain/exec.rs` | 执行档位诊断、执行计划派生、承载判定与就绪检测（纯逻辑） |
| `src/capabilities/vm/domain/guest.rs` | guest 基础根布局判定（rootfs + 内核 + 可选 initrd，纯逻辑） |
| `src/capabilities/vm/domain/packages.rs` | 运行包契约（package.yaml）校验、去重、系统路径冲突预检与来源目录（纯逻辑） |
| `src/capabilities/vm/detail/fs_packages.rs` | `PackageSource` 的磁盘实现：扫描 runtimes/ |
