# vm（虚拟机档）

> 运行包库、执行档位与计划、就绪检测与诊断；guest 端口、QEMU 后端与执行分派已就位，等真机验证。
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

**当前状态**：guest 端口、QEMU 后端与**执行分派已就位**（会话在 `tier == Vm` 时把模块命令交给本能力的 guest；建会话时按
计划与沙箱装配挂载并拉起，会话删除时销毁）。**真机验证通过前，就绪检测里的 `guest` 一项判否**，虚拟机档因此仍不可选；
现状、怎么补与验收标准见 [tests/gaps.yaml](../../tests/gaps.yaml) 的 `fence.guest-tier-not-wired`。

Linux 侧可用 `node setup-vm.js` 把 QEMU 与 Debian 基础根装进项目内 `.vm/`（只写项目内，下载源在 `links.yaml` 的 `guest.*`，需用户同意），
已据此在 TCG 下真机启动 guest、经 QGA 跑通命令；**尚缺 `/dev/kvm` 真机验收与 Windows / macOS**，就绪项仍判否。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`modules`
- 谁在用我：`cli`、`collab`、`conductor`、`session`（成员工具面带本能力的 guest 面）、入口层（`--doctor`）

## 四、改动本单元时必须同步

- `package.yaml` 契约 → [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)；本能力的用例测试在 `src/tests/vm.rs`。
- 新增端口或替身 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
