# 单元地图 · vm

> 本文是 **vm** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/vm/api.rs` | 入站面：`Vm` 用例 trait（运行包库 / 包库目录 / guest 生命周期）与档位、计划、诊断、就绪检测的重导出 |
| `src/capabilities/vm/service.rs` | 用例与端口持有者：持 `PackageSource` / `VmEngine` / `ShareBackend`（R12），按租约管理 guest |
| `src/capabilities/vm/ports.rs` | 出站端口：`PackageSource`（扫描 runtimes/ 包清单）、`VmEngine` / `GuestVm` / `GuestChannel`（起 / 跑 / 销毁 guest）与 `ShareBackend` / `SharePlan`（宿主↔guest 共享） |
| `src/capabilities/vm/domain/exec.rs` | 执行档位诊断、执行计划派生、承载判定与就绪检测（纯逻辑） |
| `src/capabilities/vm/domain/guest.rs` | guest 镜像判定（`GuestImage`）与挂载清单（纯逻辑） |
| `src/capabilities/vm/domain/paths.rs` | 宿主 ↔ guest 路径桥（恒等 / 镜像根，可逆；纯逻辑） |
| `src/capabilities/vm/domain/packages.rs` | 运行包契约（package.yaml）校验、去重、系统路径冲突预检与来源目录（纯逻辑） |
| `src/capabilities/vm/detail/fs_packages.rs` | `PackageSource` 的磁盘实现：扫描 runtimes/ |
| `src/capabilities/vm/detail/qemu.rs` | `VmEngine` / `GuestChannel` 的 QEMU 实现：拉起 qemu-system-*，经 QGA 执行与销毁（含文件搬运） |
| `src/capabilities/vm/detail/copy.rs` | `ShareBackend` 的 QGA 复制实现：建目录脚本与宿主侧 tar 打包/解包（实时共享见 gaps） |
| `src/capabilities/vm/detail/share_ninep.rs` | `ShareBackend` 的 9p 实现：-fsdev / -device 参数与 guest 端 mount 脚本 |
| `src/capabilities/vm/detail/share_virtiofs.rs` | `ShareBackend` 的 virtiofs 实现：起停 virtiofsd（vhost-user 守护进程，按进程组清理）、-chardev / vhost-user-fs-pci 参数与 guest 端 mount 脚本 |
