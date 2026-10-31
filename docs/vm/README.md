# vm（虚拟机档）

> 执行档位与计划、就绪检测与诊断、guest 生命周期；运行包契约与选包见 [packages](../packages/README.md)。
> 五轴组合（宿主 × 引擎 × guest × 通道 × 共享）可插：Linux 与 Windows 已真机验收，macOS 未验（见「五」与 `tests/macos/gaps.yaml`）。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；契约见 [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)。

## 一、管什么 / 不管什么

**管**：执行档位（本机 / 虚拟机）的选型诊断、执行计划派生（运行包由 `packages` 选好，本能力只消费 `PackagePlan`）、
**就绪检测**（本机能不能承载虚拟机档；缺哪几项、每项怎么补），以及 **guest 生命周期**（按会话租约起 / 跑 / 销毁）。

**不管**：运行包契约、包库与选包（`packages`）、会话与档案（`session`）、工作区与沙箱（`workspace`，本能力只收它给的中立路径）、模块清单（`modules`，只消费它声明的 `runtimes`）、实际执行工具（`tools`）。

## 二、入站契约与状态归属

`api::Vm`（`guest_start` / `guest_run` / `guest_wait_agent` / `guest_stop` / `guest_leases`）
+ 档位与计划的纯函数（`plan` / `tier_readiness` / `tier_refusal` / `vm_requirements` / `vm_capability` / `capability_refusal` / …）。
出站端口 `VmEngine` / `ShareBackend` **只由 `service.rs` 持有**（R12）；`GuestVm` / `GuestChannel` 由引擎在 boot 时交出。`ExecSpec` 是共享事实（在 `kernel`），本能力只读它；包库与选包在 `packages`。

### 适配器与组合

- 引擎：`QemuEngine`（唯一引擎）。
- 命令通道：`QemuHandle`（QEMU + QGA）。chardev 就绪前连上的那条会被 QEMU RST，握手失败即重连再握手——通道被换掉不是故障。
- 共享后端（用户档，默认在前）：
  - `VirtioFsShare`：QEMU vhost-user-fs，Linux 默认档。
  - `NinePShare`：挂载式共享，Linux 回退档。
  - `SmbShare`：用户态 SMB helper + guestfwd 隧道，Windows 默认档（硬前置）。
  - `CopyShare`：QGA 复制，**仅开发/应急**（`SOLOMNI_VM_SHARE=copy`），不参与用户链。
  - `NoneShare`：内部无共享档（无挂载、无同步），只给供给编排。
- 平台组合是数据（`default_combos` / `share_chain`）：共享档在**起 guest 那一刻**按候选挑，不在装配期定。
  空候选只从用户档里挑；copy 只有在显式候选（开发/应急开关）里才会被选。
- 加速器只用硬件加速：`accel=kvm` / `accel=whpx` / `accel=hvf`；**不退回软件模拟（TCG）**——软件模拟慢到没有可用性（同一台 guest 慢 2–5 倍，共享挂载会在慢启动里间歇失败），没有硬件加速时虚拟机档就绪项判否、启动如实拒绝。共享要隧道时给 `restrict=on` 网卡。
- 共享挂载（Windows 的 SMB/cifs）有界重试 3 次；仍失败才判这次起 guest 失败，并在回执里给出尝试次数与最后一次 mount 报错。smbd 自己的输出带 `[虚拟机] smbd：` 前缀转出来，不和 guest 侧现场混在一起。

### 执行分派与就绪

- 会话在 `tier == Vm` 时把模块命令交给 guest；建会话 / 启用历史会话时装配并拉起，删除会话时销毁。
  **改配按装配输入变化重建**：模块集合 / 档位 / 虚拟根 / 定版变了才重建；只改模型或权限不重建。
  网络是全局开关（`sandbox.policy.network`），改它走整体重启，不按会话重建。
- **一个主会话树共用一台 guest**：guest 租约 = 顶层工作名（proxy 树 = 代理根名），协作/节点/proxy 子工作的工具租约都指向它；
  主机围栏租约仍按子会话（`FenceSpec.lease` 与工具上下文里的 `guest_lease` 分开）。
- **环境属于主会话**：`tier / guest / net / pins` 由人建主会话时决定，子会话只读继承；核心代理的 `create_session` 不接受模型自选环境。
- **guest 装配按整棵树聚合**：plan / 沙箱 / 包冲突 / 定版取作用域根与整棵子树的并集；新子会话的包已被运行 guest 装载就直接加入，
  引入新包时要求树空闲后按新装配重建，不做运行包动态加载。
- 就绪项 `guest` 按**已验平台数据表**（`VERIFIED_HOSTS` = linux / windows）放开，并带上本平台**已实现**的组合。
- 已验组合：Linux QEMU/KVM + virtiofs（virtiofsd 不在场或**不能执行**时退 9p）；Windows QEMU/WHPX + SMB guestfwd（缺 smbd 时虚拟机档不可用，先补齐，不降级）。
- macOS 未验，缺口见 [tests/macos/gaps.yaml](../../tests/macos/gaps.yaml)。

### 判据与预检

判据分两层：

- **机器能力**（`vm_capability`：hypervisor / guest 后端 / 引擎 / 共享档）决定虚拟机档能不能**选**。
- **虚拟根是会话级**：创建或会话配置里从清单选（界面给下拉），保存 /「开始」时才合并进严格判据（`tier_readiness`）。

**静态预检**在「开始」那一刻一次走完：

- 机器能力 × 虚拟根（名字 → `.vm/guest/<名>`，`resolve_guest`）× guest 声明的运行能力（`guest.yaml` 的 `runtimes`）× 模块声明 × `packages` 的选包计划。
- 共享档按所选 guest 的 `clients` 再判：没有本平台用户档能承载的客户端就在起 guest 前拒绝，并指路 `guest.yaml`；copy 不参与这条判据（开发/应急开关除外）。
- **模块工具的入口**：相对路径入口必须在场且能跑；两档都查——虚拟机档按 **guest 的 os** 解析名字，跨平台还看文件形式（宿主编出来的产物在 guest 里跑不了，一样算缺）。
- 缺任何一项都**如实拒绝、先预检再启动**（不拉一台跑不了工具的 guest）：`packages::plan_guest` 对缺包返回问题，`diagnose_text` 给出「把包放进 `runtimes/`、或让镜像声明」的说法；`vm::plan` 只把选好的包落进执行计划。

### 并发与进程归属

- 每个主会话树一条 guest 租约（固定 `-m 2048 -smp 2`）；协作子会话与 proxy 子工作都共用这一条。
- 同时台上限是设置项 `max_guests`（默认 2，0 = 不限制），按主会话树计数，到上限如实拒绝；同一租约重启不占新名额（见 [REGISTRY_SPEC.md](../../REGISTRY_SPEC.md) 与 `domain/exec.rs` 的 `admit_guest`）。
- 会话启动时用 qemu-img 在基础镜像上建一份**每租约可写层**（`.vm/overlays/<租约>.qcow2`），删会话即丢——多台并发**不同写一份基础镜像**；供给编排直写基础镜像（它要改的就是镜像本身，`GuestSpec.isolated = false`）。
  启用 = 按基础镜像重建 guest：guest 内未落到宿主挂载点（共享区 / 沙箱 / 模块目录）的改动不保留。
- **guest 内 per-agent 围栏**：宿主把该 agent 的围栏翻成 guest 路径与允许集，经 guest 内 Linux helper（Landlock + no_new_privs）执行；
  helper 缺失或装不上时 fail-closed，不回退到无围栏执行。只支持 Linux guest；Windows / macOS guest 未适配，按不支持处理。
  本次隔离覆盖文件系统可达范围，并对齐本机 Linux 的只读基线、解释器目录、`cwd`、env 白名单、按 `GuestRun.timeout_secs` 超时杀整棵进程树与输出截断；
  同 guest 内仍是同一 UID/root，per-agent 进程信号、`/proc` 与 `system` 包的全局可见性不在承诺内。
  网络按 **per-call `GuestAccess.net`**：false 时 helper 在子进程前 `unshare(CLONE_NEWNET)`，失败 fail-closed；QEMU 网卡仍受 VM 级全局开关。
  用户在生成中按停止：host 置取消旗后经 QGA 给 guest helper 发 SIGTERM，helper 杀掉子进程组并退出；`timeout_secs` 仍照常收口。
- **稳定挂载根**：共享 guest 只挂工作树与模块树两个根，新 agent 沙箱 / 新模块目录按状态出现；具体某个 agent 能看什么由当次围栏决定。
- **运行包集合仍是启动期装配**：新子会话只加模块、不加新包时直接加入运行中的 guest；包集合变化按新计划重建共享 guest（要求树空闲）。
  `system` 包仍按 guest 全局叠写，per-agent 只保证文件可达范围，不承诺系统包隔离。
- 每个 QEMU 与共享进程起时记账（`.home/processes.json`，`kernel/detail/procs.rs`：用途 + 归属 + 标记），销毁时注销。
- 共享守护进程（`virtiofsd` / SMB helper）由对应共享后端按租约起停：起时进同一份进程账并绑父死子亡，释放时清理（`virtiofsd` 按进程组）。
- 父死子亡：核心被强杀也不留孤儿攥着镜像；三平台机制见 [docs/kernel/README.md](../kernel/README.md) 的「长驻子进程的账与回收」。
- 起停本身不在核心线程上跑（两段式见 [docs/conductor/README.md](../conductor/README.md) 的「长步骤的边界」）。
- 执行路径上 guest 表锁只用来取租约，`exec` 在该租约自己的锁下跑：**跨租约并行、同一租约串行**；工作区版本库的写闸同理按工作分。

### 依赖与虚拟根

依赖由 `provision` 能力按清单（`dependencies.yaml` + `links.yaml`，需用户同意、只写项目内）装进 `.vm/`：

- Linux：QEMU（pkgforge Anylinux AppImage）、virtiofsd 与默认虚拟根 `debian`（cloud 镜像 rootfs → `rootfs.ext4` 磁盘镜像 + 通用内核 + initrd + qemu-guest-agent，装进 `.vm/guest/debian/`；会话的可写层叠在镜像上）；构造配方按清单 `family: debian` 取，当前只有 Debian 一份。
- Windows：官方源码在 MSYS2（mingw-w64）下自建的 QEMU 11.1.1 便携包（`--enable-slirp`，固件按真实文件打包）、Debian cloud qcow2（虚拟根 `windows`，装进 `.vm/guest/windows/`）与 `guest.yaml`。
- 虚拟根的真实运行能力由 `guest-runtime` 编排自证后写进 `guest.yaml` 的 `runtimes`：Windows 走 cloud-init 首启 + 维护启动装包；Linux 的 agent 由 `guest-rootfs` 装进镜像，维护启动装 python3、C-C++ 与固定版本 Node。构造细节见 [docs/provision/README.md](../provision/README.md)。
- macOS 保持检测 + 指路（缺口见 `tests/macos/gaps.yaml`）。
- 真机探针的门槛：`SOLOMNI_VM_LIVE=1` + `SOLOMNI_VM_QEMU` / `SOLOMNI_VM_VIRTIOFSD` / `SOLOMNI_VM_SMBD` 指向 `.vm/`；虚拟根默认 `debian`，可用 `SOLOMNI_VM_GUEST` 换；默认 `[探针]` 跳过。
- 取消与超时的真机证据走**不依赖共享后端**的 `vm_live_cancel_kills_the_guest_process_tree`（`NoneShare` + guest 内 `/tmp` 围栏）：取消要 15 秒内返回、回执含「用户停止」、guest 内无残留 `sleep`；`timeout_secs` 到点回执含 helper 的「工具进程超时」。取消路径上 helper 自己的 stderr 可能被 QGA 丢掉（实测时有时无），所以判决以**无孤儿 + 快速返回**为准；超时路径的 helper stderr 稳定可见，直接断言。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`modules`
- 谁在用我：`cli`、`collab`、`conductor`、`session`（成员工具面带本能力的 guest 面）、`provision`（`guest-runtime` 的运行期编排要起 guest）、入口层（`--doctor`）

## 四、改动本单元时必须同步

- `package.yaml` 契约 → [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)；本能力的用例测试在 `src/tests/vm.rs`。
- 改虚拟根构造或运行期编排 → [docs/provision/README.md](../provision/README.md)；改 `GuestSpec.seed` / `guest_wait_agent` 要同时改那里的编排。
- 新增端口或替身 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。

## 五、多平台组合

宿主 / 引擎 / guest / 通道 / 共享**五轴都可插**——"统一 Debian guest"只是**默认实例**，不是硬约束；产品侧构造链当前只有 Debian 配方，其它 Linux 发行版可用预构造镜像 + `guest.yaml`。

| 端口 / 数据 | 职责 | 适配器（默认在前） |
| --- | --- | --- |
| `VmEngine` | 探测可用性、拉 / 停 VM、加速器、磁盘、网卡 | `QemuEngine`（三平台） |
| `GuestImage` | 镜像描述：id / arch / os / 磁盘镜像 / kernel / initrd / agent / 声明的共享客户端与运行能力 | 虚拟根 `debian`（默认，画像 id `debian-13-genericcloud-amd64`）、`windows`、用户自备 |
| `GuestChannel` | 在 guest 里执行、文件读写、销毁 | `QemuHandle`（QEMU + QGA） |
| `ShareBackend` | 宿主↔guest 共享，声明支持的引擎与 guest | `VirtioFsShare`（Linux 默认）、`NinePShare`（Linux 回退）、`SmbShare`（Windows 默认，guestfwd + 用户态 smbd helper）、`CopyShare`（仅开发/应急）、`NoneShare`（内部无共享，供给编排） |

各平台用户共享链（性能优先）：

| 宿主 | 引擎 | 共享 | 加速 | 用户链 |
| --- | --- | --- | --- | --- |
| Linux | QEMU | virtiofs（**已验**，默认；缺 virtiofsd 时退 9p） | `accel=kvm` | virtiofs → 9p |
| macOS | QEMU（未验） | 9p | `accel=hvf` | 9p |
| Windows | QEMU | smb（**已验**，硬前置） | `accel=whpx` | smb |

copy 不在任何用户链里；只有进程级开发/应急开关（`SOLOMNI_VM_SHARE=copy`）会把它作为该进程 VM 会话的唯一候选，并在日志里警告性能不可用。自定义 guest 没有声明本平台可承载的客户端时，`tier_readiness` 在起 guest 前拒绝。

- **root 只能是一份磁盘镜像**（qcow2 / raw ext4；`classify_base` 只认这个）：会话每次叠一份每租约可写层，谁也不写基础镜像；有内核就直启（`root=/dev/vda`），没有就靠镜像自带引导。需要共享时 guest 拿 `restrict=on` 的网卡（出站全断，`guestfwd` 内部隧道可用）。
- **模块构建也能落 guest**（创建会话前自动补编；手动入口是 `solomni --build=<模块 id> --guest=<虚拟根>`，见 [builds](../builds/README.md)）：模块目录以读写挂进 guest、构建期能力按 `needs` 装载，
  在 guest 里用它的编译包编，产物经挂载直接落回宿主那份模块目录（宿主工具链不进这条链）。
- **prefix 包**：`packages` 选出的 `kind: prefix` 包只读挂在 `/<prefix>`；起 guest 后把它的 `<prefix>/bin/*` 软链进 `/usr/local/bin`，模块声明的裸名命令因此在 guest 里可用。
- **system 包**：载荷按 `provides_paths` **叠写**进系统路径（复制合并 + "已叠"标记，不挂载覆盖）。
- **包库在 `packages` 单元**：契约、扫描与选包都在那里；`vm` 只消费 `PackagePlan` 做挂载/软链/叠写，不再持有 `PackageSource`。
- 就绪检测按**组合**逐项报"现状 + 怎么补"；"验过没有"是数据表（`VERIFIED_HOSTS`），不是平台 `cfg`。
- **路径桥**（`domain/paths.rs`）：宿主路径 ↔ guest 路径只有一座桥。同方言时**恒等**（宿主路径原样在 guest 里成立）；跨方言时编进镜像根（宿主 `D:\a\b` → POSIX guest `/solomni-host/d/a/b`；POSIX 宿主 → Windows guest `C:\solomni-host\…`，非法字符转义）。宿主文件系统自己的错误（例如 Windows 对 `<>` 这类非法文件名的错误码）会原样出现在 guest 的回执里——这是共享盘语义，不是 guest 内核的判定。
- 挂载点与命令装配（`cwd` / 路径型 `env`）都走路径桥；**反向也走它**：系统工具跑在宿主侧，靠它把模型给的 guest 写法译回宿主路径（`PathCodec`，见 [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 的「路径模型」）。
- **工作环境提示词按环境分发**（`prompts/shared/tools.yaml` 的 `env_hosts` / `env_guests` / `envs`）：本机档按宿主平台给，虚拟机档按 guest 的 os 给——虚拟机档给出的路径就是 guest 内路径；**系统工具与模块工具同一套写法**（路径规矩只在提示词册的 `path_rules` 里说一次，核心在寻址前翻译）。
- **虚拟根一格一个目录**（`.vm/guest/<名>/`）：名字由清单的 `guest:` 字段给，多条条目可共用一个根。
- 目录里的 `guest.yaml` 声明画像（`id` / `arch` / `os` / `agent` / `clients` / `runtimes`）：用户自备镜像自己写；产品供的由 provision 落盘（`guest-rootfs` / `guest-disk` 写基础画像），`guest-runtime` 编排再写入**逐个自证过**的 `runtimes`（Windows 另把 `clients` 写成 cifs）。
- `runtimes` 是 guest 里确实可用的运行能力，选型诊断据此不必再要 `runtimes/` 包；缺省按目录名与 Debian 默认（linux + QGA + 9p + virtiofs），不认识的值如实拒收。
- 依赖（QEMU / `virtiofsd` / SMB helper / 镜像）**不预置、不分发**，由 `provision` 能力按清单下载并解包进项目内 `.vm/`（清单 `dependencies.yaml`、下载源 `links.yaml`，需用户同意）。
- 补齐入口是**界面里的一个动作**：虚拟机档不可用、或选中的虚拟根未装齐时，配置面板与创建向导给出「补齐虚拟机依赖」（先看清单与体积、再确认下载、按进度长轮询）；CLI `solomni --provision --ensure --yes` 是没有界面时的等价路径。
- 设置里没登记 `qemu_path` / `virtiofsd_path` / `smbd_path` 时，**判据每次取用时**自动采用项目内补齐的那个可执行文件（显式登记优先）——装完即生效，不必重启。
- 共享后端也在此列：用户档在装配期持有，哪一档在起 guest 那一刻按 `share_chain` 的候选挑——装完 virtiofsd / smbd，**下一次启动就用上**。
- **开发/应急 copy**：进程启动时设 `SOLOMNI_VM_SHARE=copy`，该进程的 VM 会话强制走 copy（QGA 复制），日志打警告。它不在用户链、不进用户界面；只在开发/运维细则里出现。普通进程不装配它。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
