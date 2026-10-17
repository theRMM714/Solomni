# vm（虚拟机档）

> 运行包库、执行档位与计划、就绪检测与诊断；guest 端口、引擎后端与执行分派已就位。
> 五轴组合（宿主 × 引擎 × guest × 通道 × 共享）已落地：Linux 与 Windows 已真机验收，macOS 尚未做（见「五」与 `tests/gaps.yaml`）。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；契约见 [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)。

## 一、管什么 / 不管什么

**管**：运行包库（`runtimes/`）与 `package.yaml` 契约；执行档位（本机 / 虚拟机）的选型诊断、执行计划派生、
**就绪检测**（本机能不能承载虚拟机档；缺哪几项、每项怎么补），以及 **guest 生命周期**（按会话租约起 / 跑 / 销毁）。

**不管**：会话与档案（`session`）、工作区与沙箱（`workspace`，本能力只收它给的中立路径）、模块清单（`modules`，只消费它声明的 `runtimes`）、实际执行工具（`tools`）。

## 二、入站契约与状态归属

`api::Vm`（library / runtimes_dir / **guest_start / guest_run / guest_stop**）+ 档位与计划的纯函数
（`plan` / `tier_readiness` / `tier_refusal` / `vm_requirements` / `vm_capability` / `capability_refusal` / `vm_diagnoses` / …）。
出站端口 `PackageSource` / `VmEngine` / `ShareBackend` **只由 `service.rs` 持有**（R12）；`GuestVm` / `GuestChannel` 由引擎在 boot 时交出。`ExecSpec` 是共享事实（在 `kernel`），本能力只读它。

**当前状态**：五轴已按可插的形状落地——`QemuEngine`（当前唯一引擎）、`GuestImage`（虚拟根里可放 `guest.yaml` 声明 id / arch / os / agent / clients / runtimes）、
`QemuHandle`（QEMU + QGA，命令通道；chardev 就绪前连上的那条会被 QEMU RST，故握手失败即重连再握手——不把通道被换掉当故障）、`NinePShare`（目录根）、`CopyShare`（QGA 复制，三平台通用）、`VirtioFsShare`（QEMU vhost-user-fs，Linux/macOS 宿主）与 `SmbShare`（用户态 SMB helper + guestfwd 隧道，Windows 宿主的实时共享档）。
按平台的组合回退链是数据（`default_combos` / `share_chain`），**共享档在起 guest 那一刻按候选挑**（不是装配期定一次），加速器按平台给（`accel=kvm:tcg` / `accel=whpx:tcg` / `accel=hvf:tcg`），共享要隧道时给 `restrict=on` 网卡。
执行分派已接线（会话在 `tier == Vm` 时把模块命令交给 guest；建会话时装配并拉起，会话删除时销毁）；就绪项 `guest` 按**已验平台数据表**（`VERIFIED_HOSTS`）放开，并带上本平台上**已实现**的组合。
判据分两层：**机器能力**（`vm_capability`：hypervisor / guest 后端 / 引擎 / 共享档）决定虚拟机档能不能**选**；**虚拟根是会话级**，创建或会话配置里从清单选（界面给下拉），保存 /「开始」时才合并进严格判据（`tier_readiness`）。
**静态预检**在「开始」那一刻一次走完：机器能力 × 虚拟根（名字 → `.vm/guest/<名>`，`resolve_guest`）× guest 声明的运行能力（`guest.yaml` 的 `runtimes`）× 模块声明 × 包库 × **模块工具的入口**（相对路径入口必须在场且能跑；两档都查——虚拟机档按 **guest 的 os** 解析名字、跨平台还看文件形式，宿主编出来的产物在 guest 里跑不了，一样算缺）——
缺任何一项都**如实拒绝、先预检再启动**（不拉一台跑不了工具的 guest）：`plan` 对缺包同样返回拒绝，`diagnose_text` 给出「把包放进 `runtimes/`、或让镜像声明」的说法。

**当前状态**：Linux（QEMU/KVM + virtiofs，默认共享档；virtiofsd 不在场或**不能执行**时自动退 9p）与 Windows（QEMU/WHPX + SMB guestfwd 实时共享，缺 smbd 时自动退 QGA 复制）已真机验收——引擎级探针与 e2e 的 vm 会话场景都过
（guest 真起、工具在 guest 内执行、共享目录实时读写、删会话销毁 guest 与 virtiofsd；Windows 的镜像经 `guest-runtime` 编排预装 python3 / cifs-utils 与 NodeSource 的 node——逐个自证后才写进 guest.yaml）。
macOS 缺取包（`vm.qemu-provisioning-macos`）与 macOS Apple vz 引擎 + 自研 agent（vsock 通道）仍是缺口。契约与组合矩阵见本页「五」，缺口与验收标准见
[tests/gaps.yaml](../../tests/gaps.yaml) 的 `fence.guest-tier-not-wired`。

依赖由 `provision` 能力按清单（`dependencies.yaml` + `links.yaml`，需用户同意、只写项目内）装进 `.vm/`：Linux 装 QEMU（pkgforge Anylinux AppImage）、
virtiofsd 与默认虚拟根 `debian`（目录 rootfs + 内核 + 9p initrd + qemu-guest-agent，装进 `.vm/guest/debian/`）；Windows 装官方源码在 MSYS2（mingw-w64）下自建的 QEMU 11.1.1 便携包（`--enable-slirp`，固件按真实文件打包）+ Debian cloud qcow2（虚拟根 `windows`，装进 `.vm/guest/windows/`）+ `guest.yaml`；
虚拟根"真有什么运行能力"由 provision 能力的 `guest-runtime` 编排（Rust 侧起一次 guest，逐个自证后写进 `guest.yaml` 的 `runtimes`）确认：Windows 走 cloud-init 首启 + 维护启动装包，Linux 由 `guest-rootfs` 自带 agent、维护启动只确认 python 并尽力装 node；macOS 保持检测 + 指路。据此在 **KVM** 下跑通真机探针与产品级会话验收，在 **WHPX** 下同样跑通引擎级探针与 e2e 的 vm 会话场景（设 `SOLOMNI_VM_LIVE=1` +
`SOLOMNI_VM_QEMU` / `SOLOMNI_VM_VIRTIOFSD` / `SOLOMNI_VM_SMBD` 指向 `.vm/`（e2e 跑真机时会把自己夹具根的 `.vm` 软链到仓库的 `.vm`，跑完删掉；虚拟根与工具都在仓库那份里）（虚拟根默认 `debian`，可用 `SOLOMNI_VM_GUEST` 换），默认 `[探针]` 跳过）：guest 真启动、共享按后端进 guest（virtiofs / 9p 挂载 / SMB guestfwd / QGA 复制）、经 QGA 在 guest 里跑模块工具、删会话即销毁 guest 与 virtiofsd。

**并发**：每条会话一条租约、一台 guest（固定 `-m 2048 -smp 2`）；同时台上限是设置项 `max_guests`（默认 2，0 = 不限制），
到上限如实拒绝（同一租约重启不占新名额，见 [REGISTRY_SPEC.md](../../REGISTRY_SPEC.md) 与 `domain/exec.rs` 的 `admit_guest`）。
会话启动时用 qemu-img 在基础镜像上建一份**每租约可写层**（`.vm/overlays/<租约>.qcow2`），删会话即丢——多台并发**不同写一份基础镜像**；
供给编排仍直写基础镜像（它要改的就是镜像本身，`GuestSpec.isolated = false`）。执行路径上，guest 表锁只用来取租约，
`exec` 在该租约自己的锁下跑：**跨租约并行、同一租约串行**；工作区版本库的写闸同理按工作分。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`modules`
- 谁在用我：`cli`、`collab`、`conductor`、`session`（成员工具面带本能力的 guest 面）、`provision`（`guest-runtime` 的运行期编排要起 guest）、入口层（`--doctor`）

## 四、改动本单元时必须同步

- `package.yaml` 契约 → [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)；本能力的用例测试在 `src/tests/vm.rs`。
- 新增端口或替身 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。

## 五、多平台组合（已落地）

目标：三平台各自吃到当前最强组合，且宿主 / 引擎 / guest / 通道 / 共享**五轴都可插**——"统一 Debian guest"只是**默认实例**，不是硬约束。

| 端口 / 数据 | 职责 | 适配器（默认在前） |
| --- | --- | --- |
| `VmEngine` | 探测可用性、拉 / 停 VM、加速器、磁盘、网卡 | `QemuEngine`（三平台）、`VzEngine`（macOS，后续） |
| `GuestImage` | 镜像描述：id / arch / root（目录或磁盘镜像）/ kernel / initrd / agent / 声明的共享客户端能力 | `debian-13`（默认）、用户自备 |
| `GuestChannel` | 在 guest 里执行、文件读写、销毁 | `QemuHandle`（QEMU + QGA，默认）、`AgentVsockChannel`（后续） |
| `ShareBackend` | 宿主↔guest 共享，声明支持的引擎与 guest | 已就位：`VirtioFsShare`（Linux 默认）、`NinePShare`（回退）、`CopyShare`（三平台兜底）、`SmbShare`（Windows 默认，guestfwd + 用户态 smbd helper） |

各平台默认与回退（性能优先）：

| 宿主 | 引擎 | 共享 | 加速 | 回退链 |
| --- | --- | --- | --- | --- |
| Linux | QEMU | virtiofs（**已验**，默认；缺 virtiofsd 时退 9p） | `accel=kvm:tcg` | virtiofs → 9p → copy |
| macOS | Apple vz（后续） | 原生 virtiofs | Hypervisor.framework | vz/virtiofs → QEMU/HVF+9p → copy |
| Windows | QEMU | smb（**已验**，默认；缺 smbd 时退 copy） | `accel=whpx:tcg` | smb → copy |

- root 可为目录（QEMU 9p；必须给内核）或磁盘镜像（qcow2/raw，可自带引导）；需要共享时 guest 拿 `restrict=on` 的网卡（出站全断，`guestfwd` 内部隧道可用）。
- **模块构建也能落 guest**（`solomni --build=<模块 id> --guest=<虚拟根>`）：模块目录以读写挂进 guest、构建期能力按 `needs` 装载，
  在 guest 里用它的编译包编，产物经挂载直接落回宿主那份模块目录（宿主工具链不进这条链）。
- **运行包进 guest**：`runtimes/` 里已装载的 **prefix** 包只读挂在 `/<prefix>`，起 guest 之后把它的 `<prefix>/bin/*` 软链进 `/usr/local/bin`——模块声明的裸名命令因此直接在 guest 里可用；`kind: system` 的载荷按 `provides_paths` **叠写**进系统路径（复制合并 + "已叠"标记，不挂载覆盖）。**包库只服务虚拟机档**：本机档不装载运行包，解释器由宿主自备。
- 就绪检测按**组合**逐项报"现状 + 怎么补"；"验过没有"是数据表（`VERIFIED_HOSTS`），不是平台 `cfg`。
- **路径桥**（`domain/paths.rs`）：宿主路径 ↔ guest 路径只有一座桥——宿主与 guest 同方言时**恒等**（宿主路径原样在 guest 里成立），跨方言时编进镜像根（宿主 `D:\a\b` → POSIX guest `/solomni-host/d/a/b`；POSIX 宿主 → Windows guest `C:\solomni-host\…`，非法字符转义）。挂载点与命令装配（`cwd` / 路径型 `env`）都走它。**反向也走它**：系统工具跑在宿主侧，靠它把模型给的 guest 写法译回宿主路径（`PathCodec`，见 [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 的「路径模型」）。
- **工作环境提示词按环境分发**（`prompts/shared/tools.yaml` 的 `env_hosts` / `env_guests` / `envs`）：本机档按宿主平台给，虚拟机档按 guest 的 os 给——虚拟机档给出的路径就是 guest 内路径；**系统工具与模块工具同一套写法**（路径规矩只在提示词册的 `tool_texts.path_rules` 里说一次，核心在寻址前翻译）。
- **虚拟根一格一个目录**（`.vm/guest/<名>/`，名字由清单的 `guest:` 字段给，多条条目可共用一个根）；目录里的 `guest.yaml` 声明画像（`id` / `arch` / `agent` / `os` / `clients` / `runtimes`）：用户自备镜像自己写；产品供的由 provision 落盘（`guest-rootfs` / `guest-disk` 写基础画像），Windows 的 `guest-runtime` 编排再写入 `clients: [cifs]` 与**逐个自证过**的 `runtimes`。`runtimes` 是 guest 里确实可用的运行能力，选型诊断据此不必再要 `runtimes/` 包；缺省按目录名与 Debian 默认（linux + QGA + 9p + virtiofs），不认识的值如实拒收。
- 依赖（QEMU / `virtiofsd` / SMB helper / 镜像）**不预置、不分发**，由 `provision` 能力按清单下载并解包进项目内 `.vm/`（清单 `dependencies.yaml`、下载源 `links.yaml`，需用户同意）。
  入口是**界面里的一个动作**：虚拟机档不可用时，配置面板与创建向导给出「补齐虚拟机依赖」（先看清单与体积、再确认下载、按进度长轮询）；
  CLI `solomni --provision --ensure --yes` 是没有界面时的等价路径。设置里没登记 `qemu_path` / `virtiofsd_path` / `smbd_path` 时，
  **判据每次取用时**自动采用项目内补齐的那个可执行文件（显式登记优先）——装完即生效，不必重启。
  共享后端也在此列：全部适配器在装配期持有，哪一档在起 guest 那一刻按 `share_chain` 的候选挑——装完 virtiofsd / smbd，**下一次启动就用上**。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
