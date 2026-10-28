# 单元地图 · vm

> 本文是 **vm** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/vm/api.rs` | 入站面：`Vm` 用例 trait（guest 生命周期）与档位、计划、入口诊断、就绪检测的重导出 |
| `src/capabilities/vm/service.rs` | 用例与端口持有者：持 `VmEngine` / `ShareBackend`（R12），按租约管理 guest（每租约一把锁：跨租约并行、同一租约串行） |
| `src/capabilities/vm/ports.rs` | 出站端口：`VmEngine` / `GuestVm` / `GuestChannel`（起 / 跑 / 销毁 guest）与 `ShareBackend` / `SharePlan`（宿主↔guest 共享） |
| `src/capabilities/vm/domain/exec.rs` | 执行档位诊断、执行计划派生（消费 `PackagePlan`）、承载判定、就绪检测与并发准入（纯逻辑） |
| `src/capabilities/vm/domain/guest.rs` | guest 镜像判定（`GuestImage`）、挂载清单、prefix 软链与 system 叠写脚本、`GuestRun` 的按次超时与输出截断方式（纯逻辑） |
| `src/capabilities/vm/domain/paths.rs` | 宿主 ↔ guest 路径桥（恒等 / 镜像根，可逆；纯逻辑） |
| `src/capabilities/vm/detail/guest_helper.rs` | Linux guest 内执行助手：脚本文本、装进 guest、按 host 翻好的允许集走 Landlock + no_new_privs、按策略 `timeout_secs` 杀树；缺失/装不上 fail-closed |
| `src/capabilities/vm/detail/qemu.rs` | `VmEngine` / `GuestChannel` 的 QEMU 实现：拉起 qemu-system-*、按需用 qemu-img 建每租约可写层，经 QGA 执行与销毁（含文件搬运、按次超时与 head/head+tail 输出） |
| `src/capabilities/vm/detail/copy.rs` | `ShareBackend` 的 QGA 复制实现（仅开发/应急）：建目录脚本与宿主侧 tar 打包/解包（按趟同步；不参与用户链，实时共享是 `share_smb.rs` / `share_virtiofs.rs`） |
| `src/capabilities/vm/detail/share_ninep.rs` | `ShareBackend` 的 9p 实现：-fsdev / -device 参数与 guest 端 mount 脚本 |
| `src/capabilities/vm/detail/share_none.rs` | `ShareBackend` 的内部无共享实现：空计划、无同步，只给供给编排 |
| `src/capabilities/vm/detail/share_virtiofs.rs` | `ShareBackend` 的 virtiofs 实现：起停 virtiofsd（vhost-user 守护进程，按进程组清理）、-chardev / vhost-user-fs-pci 参数与 guest 端 mount 脚本 |
| `src/capabilities/vm/detail/share_smb.rs` | `ShareBackend` 的 SMB 实现：探测/起停用户态 smbd helper（匿名共享，凭据随租约回收）、guestfwd 隧道参数与 guest 端 mount -t cifs 脚本 |
