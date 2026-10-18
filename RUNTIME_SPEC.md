# 运行包契约（RUNTIME_SPEC）

> 本文只讲**运行包作者**要遵守的契约：一个包长什么样、怎么被声明、怎么被装配。
> 模块作者的契约见 [MODULE_SPEC.md](MODULE_SPEC.md)，分层与端口见 [ARCHITECTURE.md](ARCHITECTURE.md)，
> 产品行为见 [PRODUCT.md](PRODUCT.md)，登记处字段见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)。

## 设计立场

**模块声明能力，用户定版本，核心只认事实。**

- 模块不写版本、不写路径，只声明它要用哪个能力（`module.yaml` 的 `runtimes: [python]`）
- 精确版本由用户在会话的执行档位里定（`meta.yaml` 的 `exec.pins`）；库里同一能力有多个版本而用户没定版 = 核心如实报"需定版"，**不替用户挑**
- 包放在依赖文件夹 `runtimes/` 下：放入即出现，移出即消失——与模块清单同一个事实模型，没有注册仪式
  （该目录在 `.gitignore` 里：包内容多为二进制，只提交源码与文档）
- **缺包不是崩溃，但也不能装看不见**：本机档脚本跑不了就如实报错；虚拟机档在「开始」前**如实拒绝**（没有工具用户完不成工作），不静默降级

## 一、一个包 = 一个文件夹

```text
runtimes/<包文件夹>/
  package.yaml
  ...包内容（二进制、库、脚本，按 kind 组织，见下）
  <os>-<arch>/     # 可选：按 guest 平台分包（os ∈ linux/macos/windows，arch ∈ x86_64/aarch64）
```

**按 guest 平台分包**：包目录下放 `<os>-<arch>/` 子目录时，虚拟机档只挂与 **guest 平台**匹配的那一份（那个子目录就是包内容根，`prefix` 不变）；
有分包但没有匹配 guest 的那份 = 这个包在该 guest 上不可用，诊断如实报"缺哪个能力"，**不挂错的那份**。没有平台子目录 = 整包通用（本机档与不分平台的 guest 共用）。

`package.yaml` 字段：

```yaml
id: python                  # 能力名（模块 runtimes 引用的就是它）；同 id 不同版本可并存
version: 3.12.4             # 精确版本（不含空白与路径分隔符）
kind: prefix                # prefix（默认：挂到自己的独立前缀）| system（写进系统路径，叠层）
prefix: opt/rt/python3.12   # kind = prefix 时必填：独立前缀（书写形式：相对、/ 分隔）
provides_paths: []          # kind = system 时必填：它会写进去的系统路径
requires: []                # 本包自己需要的能力名（传递依赖由核心求闭包）
license: [PSF-2.0]          # 再分发要随包带上
note: ""                    # 说明
files:                      # 包内文件指纹：用于可复现与呈现
  - path: bin/python3
    sha256: "..."
    bytes: 12345
```

## 二、两种 kind

| kind | 含义 | 冲突 |
| --- | --- | --- |
| `prefix`（默认） | 包整体挂到自己的前缀下、只读；解释器自带相对寻址（可重定位） | 前缀之间互不干扰，除非两家声明同一个前缀，或一方的前缀目录套在另一方的里面 |
| `system` | 包会把文件写进系统路径（编译器要 `usr/include`、链接器搜索路径这类） | 与其它包写进同一处（相等或互为前缀目录）= **冲突**，装配前如实报出 |

- 冲突判定对两类包一视同仁：两条路径**相等**或**一方是另一方的前缀目录**都算落在同一处，叠加时互相覆盖，顺序消解不了。
- 装载顺序：先 `prefix` 类包（各自独立），再 `system` 类包（按 id 升序叠层）。

## 三、能力名

- 小写字母或数字开头，其余小写字母、数字、`-` `_` `.`，长度 1..=64
- 能力名就是包 id：一个包提供它自己的 id 这一个能力名（别名尚未支持）
- **产品不预置任何包**：库里有什么能力，完全取决于用户往 `runtimes/` 放了什么。
  `python` / `node` / `bash` / `cc`（C/C++ 工具链）只是常见写法的示例，没有特殊含义

## 四、指纹的定位

`files` 里的 sha256 **用于可复现，不用于信任**：用户自造的包没有可信基准，本程序不据此判断包是否安全。

当前状态：`files` 随包库扫描一起解析并保留，供呈现与后续装配使用，但**不参与装配判断**，也不做校验
（指纹对不上不会报错，也不会因此换版本）。包内容的组织方式由包作者决定，核心只认清单事实。

## 五、与执行档位的关系

两档共用一条**入口判据**（都在「开始」那一刻查）：模块工具的 `command` 首个 token 是**相对路径**（如 `./tools/run.sh`）时，它就是模块内的产物，
模块目录下必须有这个**能直接跑**的文件（unix 还要求可执行位）；不在场就如实拒绝，并指到模块自己的 README。裸名（`python` / `node` / `indexer`）不看这条，走 PATH 与运行能力。

- **本机档**（默认）：脚本直接在宿主上跑，**不装载运行包**；解释器由宿主自备，命令失败如实报错。
  **运行包库（`runtimes/`）只服务虚拟机档**：本机档不看它，也不用它满足模块声明的 `runtimes`——本机档的能力就是宿主实际有什么。
- **虚拟机档**：会话选定后，`vm` 能力做**选型诊断**并派生**装配计划**（装载顺序、虚拟根、是否放行网络）。
  `exec.net` 在虚拟机档决定 guest 网卡；本机档由平台围栏决定（Windows `internetClient`、macOS
  `network-outbound`、Linux 不围栏），默认 false，只放行出站、不授 private network / loopback。
  三类情况分开处理：
  - **装载**（只发生在虚拟机档）：**prefix** 包在起 guest 后把 `<prefix>/bin/*` 软链进 `/usr/local/bin`（裸名命令因此可用）；
    **system** 包按 `provides_paths` 把只读载荷**叠写**进系统路径（复制合并 + "已叠"标记，失败不落标记）。
  - **缺包** = 该模块的工具跑不了：虚拟机档在「开始」那一刻**如实拒绝**（不拉一台跑不了工具的 guest——
    没有工具，用户完不成工作，降级对他没有意义）。能力由两处之一满足：`runtimes/` 包库，或 guest 里可用（`guest.yaml` 的 `runtimes`；产品供给的镜像由运行期编排写入并逐个自证）
  - **选型不成立**（同一能力多版本而用户没定版 / 定版指定的版本不在库里 / 两个包写进同一处路径）=
    「开始」那一刻如实拒绝，由用户解决选型后再开始
  - **本机承载不了**（本机没有虚拟机监视器 / guest 后端未验 / 引擎或共享档不在场）= 虚拟机档**不允许创建或改入**，
    配置界面禁用并说明缺什么——这是用户环境问题，不是选型问题

虚拟机档的判据分两层：**机器能力**（虚拟机监视器 / guest 后端 / 引擎 / 共享档，`vm::vm_capability`）决定界面能不能**选**；
**虚拟根是会话级**——创建（`create_session` 的 `guest`）或会话配置里从清单选一个（界面给下拉，不让人填路径），
保存 /「开始」时与机器能力合并成严格判据（`vm::tier_readiness` / `tier_refusal`）。

当前状态：选型、诊断、装配计划、逐项前置检测、guest 生命周期与**执行分派**都已就位，并按**五轴组合**做成可插的形状——
宿主 × `VmEngine`（当前 `QemuEngine`）× `GuestImage`（虚拟根里可放 `guest.yaml` 声明 id / arch / os / agent / clients / runtimes）×
`GuestChannel`（当前 QGA）× `ShareBackend`（`VirtioFsShare` / `NinePShare` / `CopyShare` / `SmbShare`）。按平台的组合回退链是数据（`default_combos` / `choose_combo`），
加速器按平台给（`accel=kvm:tcg` / `accel=whpx:tcg` / `accel=hvf:tcg`）。**root 只有磁盘镜像一种**，会话每次叠一份每租约可写层（镜像清单见下）。
**建 / 改配 / 删除会话都分两段**：核心线程只做校验、占名与出计划，起停 guest 在调用方线程上跑，核心不陪着等 guest。
宿主路径 ↔ guest 路径只有一座桥（`domain/paths.rs`：同方言恒等，跨方言编进镜像根，**正反两向**），**挂载点、命令 `cwd` / 路径型 `env`、
工作环境提示词、系统工具的寻址四处共用它**；运行包按 guest 平台分包（`runtimes/<id>/<os>-<arch>/`）。整条业务住在独立能力 `vm`
（入口见 [docs/vm/README.md](docs/vm/README.md)，逐文件职责见 [docs/vm/unit-map.md](docs/vm/unit-map.md)）；就绪项 `guest` 按**已验平台数据表**（`VERIFIED_HOSTS`）放开，并带上本平台**已实现**的组合。

前置检测是**一份逐项清单**（机器能力 `vm::vm_requirements`，界面与 `--doctor` 照抄同一份）：

- 检查项：虚拟机监视器、guest 本体、共享档、QEMU（引擎）；虚拟根（会话级）在保存 /「开始」时合并进来。
- 每项给出"现状 + 怎么补"；共享档缺外部程序只作事实呈现、不拦档（Linux 缺 `virtiofsd` 退 9p，Windows 缺 `smbd` 退 QGA 复制）。
- 依赖（QEMU / `virtiofsd` / 镜像 / SMB helper）**不预置、不分发**（解归档的入口程序装完补可执行位——便携包里的 zip 常常不带 unix 权限位），
  由 provision 能力在 Linux / Windows 上按 `dependencies.yaml` + `links.yaml` 下载并解包进项目内 `.vm/`（需用户同意；入口 `solomni --provision --ensure --yes`；
  下载后按台账核对 sha256——自托管资产在清单条目里、链接类在 `links.yaml` 里成对，不 pin 的源如实不校验）。
- 产品检测、指路并给出补齐入口；`qemu_path` / `virtiofsd_path` / `smbd_path` 可登记。
- 虚拟根里**真有什么**由 `kind: guest-runtime` 的运行期编排（Rust 侧，不依赖仓库脚本）起一次 guest **逐个自证**后写进画像。
- Windows：首启经 cloud-init 装 `qemu-guest-agent`；维护启动装 `python3` / `cifs-utils` 与 NodeSource 的 node。
- Linux：`guest-rootfs` 把 agent 装进磁盘镜像（`.qga-ready` 标记让编排跳过首启）；维护启动装 `python3` 与 NodeSource 的 node；共享客户端保持镜像声明的 9p / virtiofs。
- 两端都是普通 ext4 磁盘根，apt / NodeSource 走标准路径。
- 两者都只把**自证过**的能力写进 `guest.yaml` 的 `runtimes`（例如 `[python, node]`）。
- 全量补齐（`--ensure --yes`）时这一条起不了 guest（缺 KVM / WHPX 这类机器事实）只如实跳过并继续；显式点名（`--ensure=vm.guest-runtime.linux`）才硬失败。

虚拟根 = 项目内 `.vm/guest/<名>` 的一格目录：

- **一份磁盘镜像**（`rootfs.ext4` / `rootfs.qcow2` / `disk.qcow2` / `disk.raw` / `rootdisk.qcow2`）——会话的每租约可写层叠在它上面。
- 可选 `vmlinuz` / `initrd.img`：有内核就直启并传 `root=/dev/vda`，没有就靠镜像自带引导。
- 可选 `guest.yaml` 声明画像（id / arch / agent / os / clients / runtimes）：`runtimes` 是 guest 里确实可用的运行能力名，如 python / node，选型诊断据此不必再要 `runtimes/` 包；缺省按目录名与 Debian 默认（os 缺省 linux），不认识的值如实拒收。
- 清单（`dependencies.yaml`）给每条 guest 条目一个 `guest:` 名，多条条目可以共同装一个虚拟根；界面读 provision 的虚拟根清单，不自己看盘。

**已真机验收**：Linux（QEMU/KVM + virtiofs，缺 `virtiofsd` 时退 9p）与 Windows（QEMU/WHPX + SMB guestfwd 实时共享；缺 `smbd` 时退 QGA 复制）——引擎级探针（`vm_live_guest_runs_command_inside_guest`）+ 并发可写层探针（`vm_live_two_guests_share_one_base_without_writing_it`）
与 e2e 的虚拟机档会话场景都过（虚拟根默认 `debian`，`SOLOMNI_VM_GUEST` 可换）；两端的 QEMU 与 guest 镜像由 provision 能力按清单装入，
Windows 的首启 / 维护启动与 Linux 的 `guest-rootfs` 打包、维护启动见上；Linux 镜像是普通 ext4 磁盘根，由 `unshare -r` + `mke2fs -d` 打包（镜像里的属主才是 root），打包后按源镜像回填特权位与 uid/gid——rdump 与 mke2fs 都拿不到它们。整段编排在 Rust 侧。
多台虚拟机档会话并发时各写自己的**每租约可写层**（不共写基础镜像），同时台上限 `max_guests`（见 [docs/vm/README.md](docs/vm/README.md) 的「并发」）。
**macOS 未验**：`VERIFIED_HOSTS` 只放开 linux / windows；缺 QEMU 取包与 Apple vz 引擎 + 自研 agent（vsock 通道）。缺口与验收标准只记在 `tests/gaps.yaml`。

## 六、校验与拒收

`package.yaml` 非法 = **拒收并说明原因**，包库报告如实列出。核心只校验，不纠正。当前拒收规则：

- `id` 不合法（不是能力名写法）；
- `version` 为空，或含空白与路径分隔符；
- `kind` 不是 `prefix` / `system`；
- `kind = prefix` 缺 `prefix`、`prefix` 不是合法书写形式（相对、`/` 分隔、无空段、无 `.` 与 `..`、不含反斜杠），
  或 `prefix` 类包却声明了 `provides_paths`；
- `kind = system` 缺 `provides_paths`，或其中任一路径不是合法书写形式；
- `requires` 里的能力名不合法，或 `requires` 依赖自己；
- 同一 `(id, version)` 出现两次：只收先出现的那份，另一份拒收并说明原因。
