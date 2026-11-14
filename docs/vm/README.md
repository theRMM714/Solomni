# vm（虚拟机档）
> 定位：执行档位的选型诊断、就绪检测与 guest 生命周期；运行包由 `packages` 选好，本单元只消费 `PackagePlan`。
> 效力：规范｜核对：2026-11-06｜依据：`src/capabilities/vm/api.rs`

## 管什么 / 不管什么
- 管：执行档位（本机 / 虚拟机）的选型诊断、执行计划派生（运行包由 `packages` 选好，本能力只消费 `PackagePlan`）、就绪检测（本机能不能承载虚拟机档；缺哪几项、每项怎么补），以及 guest 生命周期（按会话租约起 / 跑 / 销毁）与 guest 只读诊断。
- 不管：运行包契约、包库与选包（`packages`）、会话与档案（`session`）、工作区与沙箱（`workspace`，本能力只收它给的中立路径）、模块清单（`modules`，只消费它声明的 `runtimes`）、实际执行工具（`tools`）。

## 入站契约与状态归属
- 入站面：`api::Vm`（`guest_start` / `guest_run` / `guest_wait_agent` / `guest_stop` / `guest_leases` / `guest_diagnose`）+ 档位与计划的纯函数（`plan` / `tier_readiness` / `tier_refusal` / `vm_requirements` / `vm_capability` / `capability_refusal` / …）。
- 出站端口 `VmEngine` / `ShareBackend` 只由 `service.rs` 持有（R12）；`GuestVm` / `GuestChannel` 由引擎在 boot 时交出。
- `ExecSpec` 是共享形状（在 `kernel`），本能力只读它；包库与选包在 `packages`。
- 状态归属：虚拟根在项目内 `.vm/guest/<名>`；可写层在 `.vm/overlays/<租约>.qcow2`；构建缓存卷在 `.vm/build-cache/`（归 `builds`）；guest 租约按主会话树。

## 依赖图位置（由源码的 `::api` 引用推导）
- 经 `::api` 用到：`kernel`、`modules`
- 谁在用我：`cli`、`collab`、`conductor`、`session`（成员工具面带本能力的 guest 面）、`provision`（`guest-runtime` 的运行期编排要起 guest）、入口层（`--doctor`）

## 一、适配器与组合
- 引擎：`QemuEngine`（唯一引擎）。
- 命令通道：`QemuHandle`（QEMU + QGA）；chardev 就绪前连上的那条会被 QEMU RST，握手失败即重连再握手——通道被换掉不是故障。
- 共享后端（用户档，默认在前）：`VirtioFsShare`（QEMU vhost-user-fs，Linux 默认档）、`NinePShare`（挂载式共享，Linux 回退档）、`SmbShare`（用户态 SMB helper + guestfwd 隧道，Windows 默认档，硬前置）、`CopyShare`（QGA 复制，仅开发 / 应急，`SOLOMNI_VM_SHARE=copy`，不参与用户链）、`NoneShare`（内部无共享档，只给供给编排）。
- 平台组合是数据（`default_combos` / `share_chain`）：共享档在起 guest 那一刻按候选挑，不在装配期定；空候选只从用户档里挑，copy 只有在显式候选（开发 / 应急开关）里才会被选。
- 加速器只用硬件加速：`accel=kvm` / `accel=whpx` / `accel=hvf`；不退回软件模拟（TCG）——软件模拟慢到没有可用性（同一台 guest 慢 2–5 倍，共享挂载会在慢启动里间歇失败），没有硬件加速时虚拟机档就绪项判否、启动如实拒绝。
- 共享要隧道时给 `restrict=on` 网卡；共享挂载（Windows 的 SMB/cifs）有界重试 3 次，仍失败才判这次起 guest 失败，并在回执里给出尝试次数与最后一次 mount 报错；smbd 自己的输出带 `[虚拟机] smbd：` 前缀转出来，不和 guest 侧现场混在一起。

## 二、执行分派与就绪
- 会话在 `tier == Vm` 时把模块命令交给 guest；建会话 / 启用历史会话时装配并拉起，删除会话时销毁。
- 改配按装配输入变化重建：模块集合 / 档位 / 虚拟根 / 定版变了才重建；只改模型或权限不重建；网络是全局开关（`sandbox.policy.network`），改它走整体重启，不按会话重建。
- 一个主会话树共用一台 guest：guest 租约 = 顶层工作名（proxy 树 = 代理根名），协作 / 节点 / proxy 子工作的工具租约都指向它；主机围栏租约仍按子会话（`FenceSpec.lease` 与工具上下文里的 `guest_lease` 分开）。
- 环境属于主会话：`tier / guest / net / pins` 由人建主会话时决定，子会话只读继承；核心代理的 `create_session` 不接受模型自选环境。
- 网络是全局开关（`sandbox.policy.network`），不是会话字段：虚拟机档决定 guest 网卡；本机档由平台围栏决定（Windows `internetClient`、macOS `network-outbound`、Linux `net=false` 用 seccomp 拒绝非本地 socket；Linux `net=true` 不装网络过滤器，不限制入站 / 绑定）；默认 false；Windows 只放行公网出站，不授 private network / loopback。
- guest 装配按整棵树聚合：plan / 沙箱 / 包冲突 / 定版取作用域根与整棵子树的并集；新子会话的包已被运行 guest 装载就直接加入，引入新包时要求树空闲后按新装配重建，不做运行包动态加载。
- 建 / 改配 / 删除会话都分两段：核心线程只做校验、占名与出计划，起停 guest 在调用方线程上跑，核心不陪着等 guest。
- 就绪项 `guest` 按平台验收数据表（`VERIFIED_HOSTS` = linux / windows）放开，并带上本平台已实现的组合。

<!-- BEGIN:platform-status -->
> 已真机验收：Linux、Windows；未验：macOS（缺口见 `tests/macos/gaps.yaml`）。
<!-- END:platform-status -->

- 组合：Linux QEMU/KVM + virtiofs（virtiofsd 不在场或不能执行时退 9p）；Windows QEMU/WHPX + SMB guestfwd（缺 smbd 时虚拟机档不可用，先补齐，不降级）。
- Windows 验收前提：硬件加速（WHPX）可用；当前只验过物理宿主，嵌套虚拟化宿主未验，不声称支持。
- guest 内诊断入口：CLI 交互命令 `diagnose <会话>` 造一次 `guest_diagnose` 动作（`systools/tools.yaml`，`callers: [user]`，参数 `session_id` 必填非空）——只读列出 guest 进程表（pid / ppid / 状态 / CPU 占用 / RSS / 运行时长 / 命令与参数）、CPU 汇总、内存、负载、挂载与内核 / 核数；诊断本身只读、不写 guest 文件（为复用既有 fenced 通道，宿主会按既有机制写一份策略临时文件，helper 读取后立即删除）、不改会话状态、不套 per-agent 围栏，guest 内走 helper 的只读诊断模式，宿主侧只用已持有的 QGA 通道（不新增外部端口，见 `src/capabilities/vm/service.rs` 的 `guest_diagnose`）。
- 诊断失败如实分类：没有活 guest / 只有 Linux guest / QGA 通道不通 / guest 诊断助手不在或不支持 / guest 缺 python3 / 诊断超时 / 输出不可解析；「读不到」不说成「没有进程」（`src/capabilities/vm/domain/diagnose.rs`）。
- 验收证据：引擎级探针（`vm_live_guest_runs_command_inside_guest`）+ 并发可写层探针（`vm_live_two_guests_share_one_base_without_writing_it`）与 e2e 的虚拟机档会话场景都过（虚拟根默认 `debian-13`，`SOLOMNI_VM_GUEST` 可换）；外部工作区往返探针 `vm_live_external_workspace_round_trip` 与 e2e 的外部工作区场景已在 Windows 真机通过（WHPX + SMB：guest 内读外部根、写私沙落宿主、写共享区被拒、删会话不删外部根），Linux 的 virtiofs / 9p 两变体已在 KVM 上通过；两端的 QEMU 与 guest 镜像由 provision 能力按清单装入。
- 诊断的真机探针 `vm_live_guest_diagnose_lists_processes`（`SOLOMNI_VM_LIVE=1` + `SOLOMNI_VM_QEMU`，默认 `[探针]` 跳过）验：guest 里起一个忙进程后诊断能列出该 pid 与 CPU / 内存 / 负载 / 挂载，停 guest 后如实报没有活 guest。
- 多台虚拟机档会话并发时各写自己的每租约可写层（不共写基础镜像），同时台上限 `max_guests`（见「五、并发与可写层」）。
- macOS 组合：缺 QEMU 取包与 Apple vz 引擎 + 自研 agent（vsock 通道）；缺口与验收标准只记在 `tests/macos/gaps.yaml`。

## 三、判据与预检
- 判据分两层：机器能力（`vm_capability`：hypervisor / guest 后端 / 引擎 / 共享档）决定虚拟机档能不能选；虚拟根是会话级——创建（`create_session` 的 `guest`）或会话配置里从清单选（界面给下拉，不让人填路径），保存 /「开始」时才合并成严格判据（`tier_readiness` / `tier_refusal`）。
- 会话级还会检查所选 guest 的 `clients` 有没有本平台用户档能承载的，没有就在起 guest 前拒绝并指路 `guest.yaml`。
- 静态预检在「开始」那一刻一次走完：机器能力 × 虚拟根（名字 → `.vm/guest/<名>`，`resolve_guest`）× guest 声明的运行能力（`guest.yaml` 的 `runtimes`）× 模块声明 × `packages` 的选包计划。
- 共享档按所选 guest 的 `clients` 再判：没有本平台用户档能承载的客户端就在起 guest 前拒绝，并指路 `guest.yaml`；copy 不参与这条判据（开发 / 应急开关除外）。
- 外部工作区路径必须能在 guest 内表示：虚拟机档起 guest 前校验会话树根与（外部绑定时）外部工作区根都能经路径桥往返；Windows 的 UNC 路径经保留段编码后可表示（guest 内写成 `/solomni-host/unc/<server>/<share>/…`），虚拟机档因此支持 UNC 外部根；卷 GUID 形态与非 UTF-8 路径仍拒绝（没有单射的 guest 写法）。
- 模块工具的入口（`module.yaml` 的 `tools.<名>.command`）：相对路径入口必须在场且能跑；两档都查——虚拟机档按 guest 的 os 解析名字，跨平台还看文件形式（宿主编出来的产物在 guest 里跑不了，一样算缺）。
- 前置检测是一份逐项清单（机器能力 `vm::vm_requirements`，界面与 `--doctor` 照抄同一份）：虚拟机监视器、guest 本体、共享档、QEMU（引擎）；虚拟根（会话级）在保存 /「开始」时合并进来；每项给出"现状 + 怎么补"。
- 共享档是硬前置：用户共享链为空则虚拟机档不可用；Linux 缺 `virtiofsd` 仍可用 9p；Windows 缺 `smbd` 直接拒绝（先补齐，copy 不兜底）。
- 缺任何一项都如实拒绝、先预检再启动（不拉一台跑不了工具的 guest）：`packages::plan_guest` 对缺包返回问题，`diagnose_text` 给出「把包放进 `runtimes/`、或让镜像声明」的说法；`vm::plan` 只把选好的包落进执行计划。
- 本机承载不了（本机没有虚拟机监视器 / guest 后端未通过验收 / 引擎或共享档不在场）= 虚拟机档不允许创建或改入，配置界面禁用并说明缺什么——这是用户环境问题，不是选型问题。

## 四、运行包装配（消费 `PackagePlan`）
- `packages` 做选型诊断（缺包 / 定版 / 冲突），`vm` 消费它给出的 `PackagePlan` 派生装配计划（装载顺序、虚拟根）。
- 本机档：脚本直接在宿主上跑；`packages` 按宿主目标选 `prefix` 包，把包 `bin/` 前置到本次执行的 PATH，包内容根只读放行给围栏。
- 本机档就绪：运行包优先；基础能力由项目内 base profile 满足，缺失时用宿主回退——回退不是无条件可用：Windows 上还要围栏能授权（按真身安装根判：已有足够权限项、目录属主是当前用户、或当前用户对该目录有 WRITE_DAC；命令名经链接进来时容器 PATH 先换真身，链接本身不授），否则如实报不可用，不拿系统目录硬跑。
- 本机档定版只认包；多版本而没有定版 = 该模块的工具不可用并提示定版；没有包、没有 base、也没有可用回退 = 缺能力；`system` 类包不服务本机档。
- 直接动作、常驻服务、`--build` 与成员循环同一把尺子；它们没有定版入口，多版本取包库稳定序第一个；运行包集合在会话装配时固定，放入新包后重新启用会话生效。
- 虚拟机档装载（只发生在虚拟机档）：prefix 包在起 guest 后把 `<prefix>/bin/*` 软链进 `/usr/local/bin`（裸名命令因此可用）；system 包按 `provides_paths` 把只读载荷叠写进系统路径（复制合并 + "已叠"标记，失败不落标记）。
- 虚拟机档缺包 = 该模块的工具跑不了：在「开始」那一刻如实拒绝（不拉一台跑不了工具的 guest——没有工具，用户完不成工作，降级对他没有意义）；能力由两处之一满足：`runtimes/` 包库，或 guest 里可用（`guest.yaml` 的 `runtimes`；产品供给的镜像由运行期编排写入并逐个自证）。
- 选型不成立（同一能力多版本而用户没定版 / 定版指定的版本不在库里 / 两个包写进同一处路径）=「开始」那一刻如实拒绝，由用户解决选型后再开始。
- 模块工具在 guest 里只能串行（`module.yaml` 的 `tools.<名>.parallel` 不改变这一点）：同一租约一把锁、一台 guest 只有一条 QGA 命令通道；`parallel` 表达的是"可并发"的意图，在不具备并发条件的执行环境里以串行为准（不算违约）。

## 五、并发与可写层
- 每个主会话树一条 guest 租约（固定 `-m 2048 -smp 2`）；协作子会话与 proxy 子工作都共用这一条。
- 同时台上限是设置项 `max_guests`（默认 2，0 = 不限制），按主会话树计数，到上限如实拒绝；同一租约重启不占新名额（见 `REGISTRY_SPEC.md` 与 `src/capabilities/vm/domain/exec.rs` 的 `admit_guest`）。
- 会话启动时用 qemu-img 在基础镜像上建一份每租约可写层（`.vm/overlays/<租约>.qcow2`），删会话即丢——多台并发不同写一份基础镜像；供给编排直写基础镜像（它要改的就是镜像本身，`GuestSpec.isolated = false`）。
- 启用 = 按基础镜像重建 guest：guest 内未落到宿主挂载点（共享区 / 外部工作区 / 沙箱 / 模块目录）的改动不保留。
- root 只有磁盘镜像一种（qcow2 / raw ext4；`classify_base` 只认这个）：会话每次叠一份每租约可写层，谁也不写基础镜像；有内核就直启（`root=/dev/vda`），没有就靠镜像自带引导。
- 还原到初始状态：设置 → 存储空间 / `solomni --provision --reset=<虚拟根> --yes` 会删掉 `.vm/guest/<名>` 再按清单重装（不备份、不可撤销）；成功后同时清该虚拟根的构建缓存；它不是删除虚拟根：基础镜像仍会重建出来；有活跃 guest 或构建锁时拒绝。
- guest 内 per-agent 围栏：宿主把该 agent 的围栏翻成 guest 路径与允许集，经 guest 内 Linux helper（Landlock + no_new_privs）执行；helper 缺失或装不上时 fail-closed，不回退到无围栏执行。
- per-agent 围栏只支持 Linux guest；Windows / macOS guest 未适配，按不支持处理；同 guest 内仍是同一 UID/root，per-agent 进程信号、`/proc` 与 `system` 包的全局可见性不在承诺内。
- 本次隔离覆盖文件系统可达范围，并对齐本机 Linux 的只读基线、解释器目录、`cwd`、env 白名单、按 `GuestRun.timeout_secs` 超时杀整棵进程树与输出截断。只读基线里的 `/dev/null` 例外给写权（标准重定向要能丢弃输出，`2>/dev/null` 不能 EACCES）。guest 通道产出与本机同形的正文与状态行（`[shell]` 收尾四态；cwd·耗时尾行由工具层追加）：超时 / 信号由 `src/capabilities/vm/detail/guest_helper.rs` 的机器标记（`solomni-guest-fence: timeout` / `solomni-guest-fence: signal`）表达、用户停止按宿主取消旗判定，通道翻译成状态行、不写人读句（装配见 `src/kernel/domain/receipt.rs`）。
- 网络按 per-call `GuestAccess.net`：false 时 helper 在子进程前 `unshare(CLONE_NEWNET)`，失败 fail-closed；QEMU 网卡仍受 VM 级全局开关。
- 用户在生成中按停止：host 置取消旗后经 QGA 给 guest helper 发 SIGTERM，helper 杀掉子进程组并退出；`timeout_secs` 仍照常收口。
- 稳定挂载根：共享 guest 只挂三个稳定根——会话树根（各 agent 私沙与 `.work` 的父目录，rw）、外部工作区根（仅根会话绑定外部工作区时，rw）、模块树根（rw）；内部布局下共享区在会话树内、不重复挂，外部绑定时会话树根与外部根互不包含、各挂一条。新 agent 沙箱按会话树根出现、新模块目录按模块树根出现，不逐项枚举挂载；具体某个 agent 能看什么由当次围栏决定。外部工作区根必须可写：`src/capabilities/vm/service.rs` 会把只读挂载无条件并入每次调用的允许集，只读会绕过 per-agent 的共享区读取范围；guest 不直接写共享区，写共享区走宿主侧 `work_commit`。
- 运行包集合仍是启动期装配：新子会话只加模块、不加新包时直接加入运行中的 guest；包集合变化按新计划重建共享 guest（要求树空闲）；`system` 包仍按 guest 全局叠写，per-agent 只保证文件可达范围，不承诺系统包隔离。
- 每个 QEMU 与共享进程起时记账（`.home/` 的 `processes.json`，`src/kernel/detail/procs.rs`：用途 + 归属 + 标记），销毁时注销。
- 共享守护进程（`virtiofsd` / SMB helper）由对应共享后端按租约起停：起时进同一份进程账并绑父死子亡，释放时清理（`virtiofsd` 按进程组）。
- 父死子亡：核心被强杀也不留孤儿攥着镜像；三平台机制见 `docs/kernel/README.md` 的「长驻子进程的账与回收」。
- 起停本身不在核心线程上跑（两段式见 `docs/conductor/README.md` 的「长步骤的边界」）。
- 执行路径上 guest 表锁只用来取租约，`exec` 在该租约自己的锁下跑：跨租约并行、同一租约串行；工作区版本库的写闸同理按工作分。

## 六、依赖、虚拟根与运行期自证
- 依赖由 `provision` 能力按清单（`dependencies.yaml` + `links.yaml`，需用户同意、只写项目内）装进 `.vm/`。
- 依赖（QEMU / `virtiofsd` / SMB helper / 镜像）不预置、不分发（解归档的入口程序装完补可执行位——便携包里的 zip 常常不带 unix 权限位），由 provision 能力在 Linux / Windows 上按 `dependencies.yaml` + `links.yaml` 下载并解包进项目内 `.vm/`（需用户同意；入口 `solomni --provision --ensure --yes`；下载后按台账核对 sha256——自托管资产在清单条目里（`release:<tag>/<资产>` + `sha256:`）、链接类在 `links.yaml` 里成对，不 pin 的源如实不校验）。
- 产品检测、指路并给出补齐入口；`qemu_path` / `virtiofsd_path` / `smbd_path` 可登记。
- Linux：QEMU（pkgforge Anylinux AppImage）、virtiofsd 与默认虚拟根 `debian-13`（cloud 镜像 rootfs → `rootfs.ext4` 磁盘镜像 + 通用内核 + initrd + qemu-guest-agent，装进 `.vm/guest/debian-13/`；会话的可写层叠在镜像上）；构造配方按清单 `family: debian` 取，当前只有 Debian 一份。
- Windows：官方源码在 MSYS2（mingw-w64）下自建的 QEMU 11.1.1 便携包（`--enable-slirp`，固件按真实文件打包）、Debian cloud qcow2（与 Linux 同一个虚拟根 `debian-13`，装进 `.vm/guest/debian-13/`）与 `guest.yaml`。
- 虚拟根里真有什么由 `kind: guest-runtime` 的运行期编排（Rust 侧，不依赖仓库脚本）起一次 guest 逐个自证后写进画像。
- 两端都只把自证过的能力写进 `guest.yaml` 的 `runtimes`（例如 `[python, node, cc, gcc]`；`gcc` 是 `build.needs` 用的可执行名）。
- Windows：首启经 cloud-init 装 `qemu-guest-agent`；维护启动装 `python3` / `cifs-utils` / C-C++（`gcc` `g++` `make` `libc6-dev`），并从固定版本官方二进制装 Node。
- Linux：`guest-rootfs` 把 agent 装进磁盘镜像（`.qga-ready` 标记让编排跳过首启）；维护启动装 `python3` 与 C-C++，并从固定版本官方二进制装 Node；共享客户端保持镜像声明的 9p / virtiofs。
- 两端都是普通 ext4 磁盘根；C/C++ 走 Debian apt 标准路径，Node 走官方发行包的固定 URL + sha256。
- Linux 镜像是普通 ext4 磁盘根，由 `unshare -r` + `mke2fs -d` 打包（镜像里的属主才是 root），打包后按源镜像回填特权位与 uid/gid——rdump 与 mke2fs 都拿不到它们；整段编排在 Rust 侧。
- 全量补齐（`--ensure --yes`）时这一条起不了 guest（缺 KVM / WHPX 这类机器事实）只如实跳过并继续；显式点名（`--ensure=vm.guest-runtime.linux`）才硬失败。
- 虚拟根 = 项目内 `.vm/guest/<名>` 的一格目录：一份磁盘镜像（`rootfs.ext4` / `rootfs.qcow2` / `disk.qcow2` / `disk.raw` / `rootdisk.qcow2`）——会话的每租约可写层叠在它上面；可选 `vmlinuz` / `initrd.img`（有内核就直启并传 `root=/dev/vda`，没有就靠镜像自带引导）。
- 一格目录里的 `guest.yaml` 声明画像（`id` / `arch` / `os` / `agent` / `clients` / `runtimes`）：`runtimes` 是 guest 里确实可用的运行能力名，如 python / node，选型诊断据此不必再要 `runtimes/` 包；缺省按目录名与 Debian 默认（os 缺省 linux），不认识的值如实拒收。
- 清单（`dependencies.yaml`）给每条 guest 条目一个 `guest:` 名，多条条目可以共同装一个虚拟根；名字按 guest 的「发行版-大版本」取（`debian-13`），不按宿主平台取——两个宿主装的是同一个 Debian 13；界面读 provision 的虚拟根清单，不自己看盘。
- 真机探针的门槛：`SOLOMNI_VM_LIVE=1` + `SOLOMNI_VM_QEMU` / `SOLOMNI_VM_VIRTIOFSD` / `SOLOMNI_VM_SMBD` 指向 `.vm/`；虚拟根默认 `debian-13`，可用 `SOLOMNI_VM_GUEST` 换；外部工作区往返探针 `vm_live_external_workspace_round_trip` 另按平台要求共享工具（Windows 要 `SOLOMNI_VM_SMBD`；Linux 要 KVM + `SOLOMNI_VM_VIRTIOFSD` 跑 virtiofs 变体，不设它跑 9p 变体，两条各跑一次）；默认 `[探针]` 跳过。共享边界另有两条只读验收：`src/tests/vm/share_boundary.rs` 的 `smb_read_only_share_rejects_guest_writes`（Windows + SMB：只读写不进、可写落回宿主）与 `ninep_read_only_share_rejects_guest_writes`（非 Windows + 9p；Windows 宿主没有这条生产路径，如实 env-skip，缺口见 `tests/windows/gaps.yaml`）。
- 取消与超时的真机证据走不依赖共享后端的 `vm_live_cancel_kills_the_guest_process_tree`（`NoneShare` + guest 内 `/tmp` 围栏）：取消要 15 秒内返回、回执含 `[shell] 用户停止`、guest 内无残留 `sleep`；`timeout_secs` 到点回执含 `[shell] 超时（2 秒）`（用例给 2 秒）；直调通道只到状态行，cwd·耗时尾行由工具层在端到端 shell 回执里追加（见 `docs/tools/tools-and-roles.md` 的「执行类系统工具：shell」）；判决以无孤儿 + 快速返回为准。
- macOS 保持检测 + 指路（缺口见 `tests/macos/gaps.yaml`）；产品侧构造链当前只有 Debian 配方，其它 Linux 发行版可用预构造镜像 + `guest.yaml`。

## 七、多平台组合与路径桥
- 宿主 / 引擎 / guest / 通道 / 共享五轴都可插——"统一 Debian guest"只是默认实例，不是硬约束。
- 端口职责（适配器默认在前）：`VmEngine` 探测可用性、拉 / 停 VM、加速器、磁盘、网卡 → `QemuEngine`（三平台）；`GuestImage` 镜像描述：id / arch / os / 磁盘镜像 / kernel / initrd / agent / 声明的共享客户端与运行能力 → 虚拟根 `debian-13`（默认，画像 id `debian-13-genericcloud-amd64`）、用户自备；`GuestChannel` 在 guest 里执行、文件读写、销毁 → `QemuHandle`（QEMU + QGA）；`ShareBackend` 宿主↔guest 共享，声明支持的引擎与 guest → `VirtioFsShare`（Linux 默认）、`NinePShare`（Linux 回退）、`SmbShare`（Windows 默认，guestfwd + 用户态 smbd helper）、`CopyShare`（仅开发 / 应急）、`NoneShare`（内部无共享，供给编排）。
- 各平台用户共享链（性能优先）：Linux QEMU + virtiofs（默认；缺 virtiofsd 时退 9p）+ `accel=kvm`，链路 virtiofs → 9p；macOS QEMU（未通过验收，缺口见 `tests/macos/gaps.yaml`）+ 9p + `accel=hvf`；Windows QEMU + smb（已验，硬前置）+ `accel=whpx`。
- copy 不在任何用户链里；只有进程级开发 / 应急开关（`SOLOMNI_VM_SHARE=copy`）会把它作为该进程 VM 会话的唯一候选，并在日志里警告性能不可用；自定义 guest 没有声明本平台可承载的客户端时，`tier_readiness` 在起 guest 前拒绝。
- 模块构建也能落 guest（创建会话前自动补编；手动入口是 `solomni --build=<模块 id> --guest=<虚拟根>`），细节见 `docs/builds/README.md`。
- 包库在 `packages` 单元：契约、扫描与选包都在那里；`vm` 只消费 `PackagePlan` 做挂载 / 软链 / 叠写，不再持有 `PackageSource`。
- 就绪检测按组合逐项报"现状 + 怎么补"；"验过没有"是数据表（`VERIFIED_HOSTS`），不是平台 `cfg`。
- 路径桥（`src/capabilities/vm/domain/paths.rs`）：宿主路径 ↔ guest 路径只有一座桥；同方言时恒等（宿主路径原样在 guest 里成立）；跨方言时编进镜像根（宿主 `D:\a\b` → POSIX guest `/solomni-host/d/a/b`；POSIX 宿主 → Windows guest `C:\solomni-host\…`，非法字符转义）；UNC 根编成保留段（宿主 `\\server\share\a` → `/solomni-host/unc/server/share/a`，盘符段只可能是单字母，两种段不会撞）；宿主文件系统自己的错误（例如 Windows 对 `<>` 这类非法文件名的错误码）会原样出现在 guest 的回执里——这是共享盘语义，不是 guest 内核的判定。
- 挂载表优先：宿主路径落在某个已挂稳定根 / 包根内时用该挂载的 guest 点，表外才退镜像根；外部工作区根同样由桥给挂载点（内部布局下共享区在会话树内，不额外挂）。
- 挂载点与命令装配（`cwd` / 路径型 `env`）都走路径桥；反向也走它：系统工具跑在宿主侧，靠它把模型给的 guest 写法译回宿主路径（`PathCodec`，见 `docs/tools/tools-and-roles.md` 的「路径模型」）。起 guest 前校验每个稳定根都能经桥往返（`check_stable_roots`）；UNC 经保留段编码后往返成立（虚拟机档因此支持 UNC 外部根），卷 GUID 与非 UTF-8 如实拒绝。
- 工作环境提示词按环境分发（`prompts/shared/tools.yaml` 的 `env_hosts` / `env_guests` / `envs`）：本机档按宿主平台给，虚拟机档按 guest 的 os 给——虚拟机档给出的路径就是 guest 内路径；系统工具与模块工具同一套写法（路径规矩只在提示词册的 `path_rules` 里说一次，核心在寻址前翻译）。
- 补齐入口是界面里的一个动作：虚拟机档不可用、或选中的虚拟根未装齐时，配置面板与创建向导给出「补齐虚拟机依赖」（先看清单与体积、再确认下载、按进度长轮询）；CLI `solomni --provision --ensure --yes` 是没有界面时的等价路径。
- 设置里没登记 `qemu_path` / `virtiofsd_path` / `smbd_path` 时，判据每次取用时自动采用项目内补齐的那个可执行文件（显式登记优先）——装完即生效，不必重启。
- 共享后端也在此列：用户档在装配期持有，哪一档在起 guest 那一刻按 `share_chain` 的候选挑——装完 virtiofsd / smbd，下一次启动就用上。
- 开发 / 应急 copy：进程启动时设 `SOLOMNI_VM_SHARE=copy`，该进程的 VM 会话强制走 copy（QGA 复制），日志打警告；它不在用户链、不进用户界面；只在开发 / 运维细则里出现，普通进程不装配它。

## 改动本单元时必须同步
- `RUNTIME_SPEC.md` → 包契约；`docs/builds/README.md` → 构建；本能力的用例测试在 `src/tests/vm/`。
- 改虚拟根构造或运行期编排 → `docs/provision/README.md`；改 `GuestSpec.seed` / `guest_wait_agent` 要同时改那里的编排。
- 新增端口或替身 → `docs/testing/doubles.md` 的端口矩阵。
- `REGISTRY_SPEC.md` → `max_guests` 等设置字段；`docs/tools/tools-and-roles.md` → 路径模型与系统工具寻址。

## 本目录
| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
