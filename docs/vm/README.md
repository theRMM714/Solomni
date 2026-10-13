# vm（虚拟机档）

> 运行包库、执行档位与计划、就绪检测与诊断；guest 端口、QEMU 后端与执行分派已就位。
> 多平台组合（宿主 × 引擎 × guest × 通道 × 共享）为**已批准方向，实施中**（见「五」）。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；契约见 [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)。

## 一、管什么 / 不管什么

**管**：运行包库（`runtimes/`）与 `package.yaml` 契约；执行档位（本机 / 虚拟机）的选型诊断、执行计划派生、
**就绪检测**（本机能不能承载虚拟机档；缺哪几项、每项怎么补），以及 **guest 生命周期**（按会话租约起 / 跑 / 销毁）。

**不管**：会话与档案（`session`）、工作区与沙箱（`workspace`，本能力只收它给的中立路径）、模块清单（`modules`，只消费它声明的 `runtimes`）、实际执行工具（`tools`）。

## 二、入站契约与状态归属

`api::Vm`（library / runtimes_dir / **guest_start / guest_run / guest_stop**）+ 档位与计划的纯函数
（`plan` / `tier_readiness` / `tier_refusal` / `vm_requirements` / `vm_diagnoses` / …）。
出站端口 `PackageSource` / `GuestHost` **只由 `service.rs` 持有**（R12）。`ExecSpec` 是共享事实（在 `kernel`），本能力只读它。

**当前状态**：单一 Debian guest、QEMU 后端、9p 共享与**执行分派已就位**（会话在 `tier == Vm` 时把模块命令交给本能力的 guest；
建会话时按计划与沙箱装配挂载并拉起，会话删除时销毁）；guest 有 `-nic none`，root 走 9p，就绪项 `guest` 仅 Linux 放行。

**已批准、实施中**：把 guest 做成四轴可扩展组合——宿主 × `VmEngine`（默认 QEMU；macOS 后续 Apple vz）× `GuestImage`
（默认 Debian，后续用户自备）× `GuestChannel`（默认 QGA；后续自研 agent + vsock）× `ShareBackend`（virtiofs / 9p / SMB / copy），
各平台默认吃当前最强组合并逐级回退；root 改为磁盘镜像。契约与组合矩阵见本页「五」，现状与验收见
[tests/gaps.yaml](../../tests/gaps.yaml) 的 `fence.guest-tier-not-wired`。

Linux 侧可用 `node setup-vm.js` 把 QEMU 与 Debian 基础根装进项目内 `.vm/`（只写项目内，下载源在 `links.yaml` 的 `guest.*`，需用户同意）；
据此在 **KVM** 下跑通真机探针与产品级会话验收（设 `SOLOMNI_VM_LIVE=1` + `SOLOMNI_VM_QEMU` / `SOLOMNI_VM_BASE` 指向 `.vm/`，默认 `[探针]` 跳过）：
guest 真启动、额外 9p 共享挂进 guest、经 QGA 在 guest 里跑模块工具、删会话即销毁 guest；**Windows / macOS 尚未真机验收**。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`modules`
- 谁在用我：`cli`、`collab`、`conductor`、`session`（成员工具面带本能力的 guest 面）、入口层（`--doctor`）

## 四、改动本单元时必须同步

- `package.yaml` 契约 → [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)；本能力的用例测试在 `src/tests/vm.rs`。
- 新增端口或替身 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。

## 五、多平台组合（已批准，实施中）

目标：三平台各自吃到当前最强组合，且引擎 / 通道 / 共享 / guest 四轴都可插——"统一 Debian guest"只是**默认实例**，不是硬约束。

| 端口 / 数据 | 职责 | 适配器（默认在前） |
| --- | --- | --- |
| `VmEngine` | 探测可用性、拉 / 停 VM、加速器、磁盘、网卡 | `QemuEngine`（三平台）、`VzEngine`（macOS，后续） |
| `GuestImage` | 镜像描述：id / arch / root 磁盘镜像 / kernel / initrd / agent / 声明的共享客户端能力 | `debian-13`（默认）、用户自备（后续） |
| `GuestChannel` | 在 guest 里执行、文件读写、销毁 | `QgaChannel`（QEMU virtio-serial，默认）、`AgentVsockChannel`（后续） |
| `ShareBackend` | 宿主↔guest 共享，声明支持的引擎与 guest | 已就位：`NinePShare`、`CopyShare`；待做：`VirtioFsShare`、`SmbShare` |

各平台默认与回退（性能优先）：

| 宿主 | 引擎 | 共享 | 加速 | 回退链 |
| --- | --- | --- | --- | --- |
| Linux | QEMU | virtiofs（需 `virtiofsd`） | `kvm:tcg` | virtiofs → 9p → smb → copy |
| macOS | Apple vz（阶段二） | 原生 virtiofs | Hypervisor.framework | vz → QEMU/HVF+9p → smb → copy |
| Windows | QEMU | SMB over guestfwd | `whpx:tcg` | smb → copy |

- root 一律磁盘镜像（qcow2/raw），不再用共享文件系统当根；guest 有 `restrict=on` 的网卡（出站全断，`guestfwd` 内部隧道可用）。
- 就绪检测按**组合**逐项报"现状 + 怎么补"；"验过没有"是组合台账（数据），不再用平台 `cfg`。
- 依赖（QEMU / `virtiofsd` / 镜像）用户自备，由 `setup-vm.js` 按平台装进项目内 `.vm/`；下载源一律进 `links.yaml`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
