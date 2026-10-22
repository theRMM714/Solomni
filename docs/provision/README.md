# provision（依赖补齐）

> 把"不预置、不分发"的依赖按清单下载、解包进产品内，并管一次补齐作业。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：内嵌 `dependencies.yaml`（依赖清单）与 `links.yaml`（下载源台账）、本机状态（缺什么、装到哪、从哪来）、下载缓存（`.vm/cache`）、解包落盘（`kernel::detail::unpack`）、一次后台补齐作业（进度 / 取消），以及**本机有哪些长驻子进程在跑**（读 `kernel::detail::procs` 的账：用途、归属、起止）。

**不管**：虚拟机档的能力语义与档位判定（在 `vm`）；构建期工具链（本机档由宿主自备，虚拟机档由 guest 的运行能力提供——都不是本能力）；界面怎么展示（在 `presentation`）。

## 二、入站契约与状态归属

- 入站契约在 [`src/capabilities/provision/api.rs`](../../src/capabilities/provision/api.rs)：`Provision`（`status` / `guest_bases` / `job` / `start` / `cancel`）。
- DTO：`DepStatus` / `GuestBase` / `JobState`；`NoProvision` 是未接入的默认实现（如实失败，不假装可用）。
- **虚拟根清单**：`guest_bases` 把清单里带 `guest:` 的条目按虚拟根名归并（`ready` = 这一根的各条都在场），一个虚拟根可由多条条目共同装；conductor 的创建向导与会话配置按它渲染下拉。
- 状态归属：清单与台账的唯一真相在仓库根 `dependencies.yaml` / `links.yaml`，产品里**编译期内嵌**；作业状态是**进程内**的（不落盘，进程重启即清空）。

### 安装方式（清单 `kind`）

| `kind` | 做什么 |
| --- | --- |
| `archive`（默认） | 解归档到 `install`。 |
| `appimage` | 自带 `--appimage-extract`，不依赖 FUSE；保持包内原布局（applet 按 `argv[0]` 找 `../shared/bin/<名>`）。 |
| `file` | 单文件直下，按 `probe` 的文件名落盘。 |
| `guest-disk` | 单文件（Debian cloud qcow2）+ `guest.yaml` 基础画像；镜像已在场时**不覆盖**（运行期状态在里面）。 |
| `guest-rootfs` | Linux 虚拟根多步构造，见下。 |
| `guest-runtime` | 虚拟根的运行期编排，见下。 |

解归档的入口程序（清单 `probe` 指到的那个）装完会补**可执行位**：zip 不一定带 unix 权限位，解出来会是不能跑的 0644（例如 `virtiofsd` 的便携包）。

### guest-rootfs：Linux 虚拟根的多步构造

一条 `guest-rootfs` 条目按顺序做这些事，只写项目内：

1. 缓存里解开 cloud 镜像 tar，得到 `disk.raw`（GNU sparse，用系统 `tar`）。
2. `sfdisk` 读 GPT 取 root 分区，`dd` 切出 `rootfs.ext4`。
3. `debugfs rdump` 把 rootfs 解成目录树。**此时先读出一份"打包会丢"的属性清单**（特权位 setuid / setgid / sticky、非 root 属主与属组）——随后要装内核与 agent，目录树不再与源镜像一一对应。
4. 装通用内核。cloud 镜像自带内核的 `CONFIG_NET_9P` 没编（`# CONFIG_NET_9P is not set`），共享回退档 9p 就用不了；所以只在"已装 ABI 真带 `9pnet_virtio` 模块"时沿用，否则装与 rootfs 同 Debian 大版本的**普通**通用内核（排除 `-rt-` / `-cloud-`）。
5. 装 `qemu-guest-agent`。
6. 清掉 cloud 镜像留下的 `fstab` 与 remount 单元（root 由内核命令行给出），用第 4 步选定的 ABI 生成 initrd（带 9p 模块），并把 `vmlinuz` / `initrd.img` 两个稳定软链换指。
7. 写基础 `guest.yaml` 画像。
8. `unshare -r` + `mke2fs -d` 把目录树打包成 `rootfs.ext4` 磁盘镜像（4 KiB 块）；镜像里的属主才是 root。
9. 在镜像上**回填第 3 步读出的属性**并核对：rdump / mke2fs 拿不到特权位与 uid/gid。
10. 写 `.qga-ready`（agent 已进镜像），删掉目录中间产物，只留镜像 + 内核 + initrd + `guest.yaml`。

调用的系统工具：`tar` / `sfdisk` / `dd` / `debugfs` / `ar` / `unshare` / `mke2fs`。

重跑时：缓存里的 `disk.raw` / `rootfs.ext4` 与已解出的目录树直接复用；打新镜像前先删掉上一次的 `rootfs.ext4`（否则那份旧镜像会被一起打进新镜像）。

### guest-runtime：虚拟根运行期编排

经 `vm::api` 起一次 guest，装运行包并**逐个自证**，只把自证过的能力写进 `guest.yaml` 的 `runtimes`：

- Windows 虚拟根：首启经 cloud-init 种子装 `qemu-guest-agent`，落 `.qga-ready`。
- Linux 虚拟根：agent 由 `guest-rootfs` 装进镜像，`.qga-ready` 让编排跳过首启；维护启动先按 QEMU 用户网络配网。
- 维护启动两端同一条路：整份刷新 apt lists、装 python3 与 C-C++（gcc / g++ / make / libc6-dev），并从固定版本官方二进制装 Node（Windows 另加 cifs-utils，客户端声明写成 cifs；Linux 保持镜像声明的 9p/virtiofs）。
- 自证命令是 `python -V`、`node --version` 与一次最小 C++ 编译：三者都通了才声明；通过后写 `.runtimes-ready`，失败不写（下次重试）。
- 当前基础镜像按 2.3 GB 构造（留 1.5 GB 余量）；不是这个基线的虚拟根**不在支持范围**，不自动修补——删除 `.vm/guest/<名>` 后重新 `--provision --ensure --yes`。
- 全量补齐时这一条失败只如实跳过并继续；显式点名才硬失败——它依赖本机能起 guest。

### 缓存与完整性

- 缓存目录 `.vm/cache`。缓存里那份（复用的、刚下好的）进解包前按台账核对 sha256：
  - 自托管 release 资产读清单条目（`sha256:`）；
  - 链接类源读 `links.yaml` 的 `<键>.sha256`（pin 与它守护的 URL 在同一份台账里）；
  - 不 pin 的源如实不校验，靠解包 / 解析成功判。
- 不符时**不删缓存**：报出台账值、实际值与来源——删了会在 pin 过期时变成每次重下的死循环。

### 补齐后的生效方式

- 主程序**每次判据取用时**自动采用项目内补齐的可执行文件（`qemu_path` / `virtiofsd_path` / `smbd_path` 未登记时；显式登记优先，取到的是拼接产品根后的绝对路径）——装完即生效，不需要重启进程。
- 共享后端全部在装配期持有，哪一档在起 guest 那一刻按候选挑（见 [docs/vm/README.md](../vm/README.md) 的「多平台组合」）。

### 作业与进度

- 作业状态带单调 `seq`；`GET /api/setup/status?since=N&wait=1` 是长轮询（最多 20s），拿到新行即回，前端按 `seq` 增量取。
- **进度只有这一条通道**：`seq` + `lines`（留最近 200 行）——不另存"当前步骤"摘要，界面要看当前步骤就取最后一行。
- **要装什么由后端定**：`status` 的 `pending` 列出还没在场的条目 id；`ensure` 不点名时只补这些（执行核心不跳过在位项，不滤就会把已装的重下一遍、重解一遍），都装好了直接回 `{ok,deps:[]}` 且不起作业。显式点名 = 逐条失败硬报（不是全量补齐那种"跳过继续"）。
- 呈现层：**Web 的「补齐虚拟机依赖」弹层**——配置面板与创建向导在虚拟机档不可用时给这个动作：先看清单与体积、再确认下载、然后按 `seq` 长轮询进度、最后如实说清生效范围。
- 无界面时的等价路径：CLI `--provision`（`src/diagnostics/`，隐藏模式，`--ensure[=id,…] --yes` 才真下载）。路由表见 [docs/presentation/contracts.md](../presentation/contracts.md)。

## 三、依赖图位置（由源码的 ::api 引用推导）

- 经 `::api` 用到：`vm`（`guest-runtime` 的运行期编排要起 guest：`guest_start` / `guest_wait_agent` / `guest_run` / `guest_stop`）；另经 kernel 的机制 `outbound` / `unpack`。
- 谁在用我：`conductor`（`Ops` 的 `provision` 字段与 `GuestBase` 等类型）、`presentation`（setup 路由）、`src/diagnostics/`（`--provision`）。

## 四、改动本单元时必须同步

- 改清单字段或新增依赖 → `dependencies.yaml` 与 `dependencies/THIRD-PARTY-NOTICES.md`（自托管包的许可证与对应源码）；需要新下载源时改 `links.yaml`。
  **自托管 release 资产要连 `sha256:` 一起登记**（值取该 release 资产的 digest，来源写进 `note`）；链接类源的校验值写进 `links.yaml` 的 `<键>.sha256`。
  升级资产时 URL 与校验值必须同一个 diff 改掉。
- 改入站契约或路由 → `docs/presentation/contracts.md` 的 ROUTES 表与 `src/tests/routes.rs`（契约测试机器比对）。
- 改下载 / 解包机制 → kernel 的 `outbound` / `unpack` 单元地图。
- 改 guest 运行期编排 → [docs/vm/README.md](../vm/README.md) 与 vm 的 `GuestSpec.seed` / `guest_wait_agent` 契约。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |