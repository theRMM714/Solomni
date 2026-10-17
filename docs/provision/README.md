# provision（依赖补齐）

> 把"不预置、不分发"的依赖按清单下载、解包进产品内，并管一次补齐作业。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：内嵌 `dependencies.yaml`（依赖清单）与 `links.yaml`（下载源台账）、本机状态（缺什么、装到哪、从哪来）、下载缓存（`.vm/cache`）、解包落盘（`kernel::detail::unpack`）与一次后台补齐作业（进度 / 取消）。

**不管**：虚拟机档的能力语义与档位判定（在 `vm`）；构建期工具链（开发侧自备）；界面怎么展示（在 `presentation`）。

## 二、入站契约与状态归属

- 入站契约在 [`api.rs`](api.rs)：`Provision`（`status` / `guest_bases` / `job` / `start` / `cancel`）+ DTO（`DepStatus` / `GuestBase` / `JobState`）；`NoProvision` 是未接入的默认实现（如实失败，不假装可用）。
- **虚拟根清单**：`guest_bases` 把清单里带 `guest:` 的条目按虚拟根名归并（`ready` = 这一根的各条都在场），一个虚拟根可由多条条目共同装；conductor 的创建向导与会话配置按它渲染下拉。
- 安装方式（清单 `kind`）：`archive`（默认，解归档）/ `appimage`（自带 `--appimage-extract`，不依赖 FUSE）/ `file`（单文件直下，按 probe 的文件名落盘）/ `guest-disk`（单文件 + `guest.yaml` 画像）/ `guest-rootfs`（Linux guest 虚拟根多步构造：cloud 镜像 → rootfs + **带 9p 的通用内核** + 9p initrd + qemu-guest-agent，并写一份基础 `guest.yaml` 画像；调 `tar` / `sfdisk` / `dd` / `debugfs` / `ar` / `unshare`，只写项目内。**内核不能拿 cloud 镜像自带那个**——它的 `CONFIG_NET_9P` 没编（`# CONFIG_NET_9P is not set`），起不了 9p 根；所以只在"已装 ABI 真带 `9pnet_virtio` 模块"时沿用，否则装与 rootfs 同 Debian 大版本的**普通**通用内核（排除 `-rt-` / `-cloud-`），并把 `vmlinuz` / `initrd.img` 两个稳定软链换指）/ `guest-runtime`（虚拟根的运行期编排：经 `vm::api` 起一次 guest 装运行包并**逐个自证**（`python -V` / `node --version`），只把自证过的能力写进 `guest.yaml` 的 `runtimes`，并落 `.runtimes-ready` 标记；只写项目内。**磁盘根**：首启经 cloud-init 种子装 `qemu-guest-agent`，维护启动整份刷新 apt lists、装 python3 / cifs-utils 与 NodeSource 的 nodejs，客户端声明写成 cifs。**目录根（Linux）**：agent 由 `guest-rootfs` 装好、客户端保持 9p/virtiofs；它不走 cloud-init，所以维护启动先**配网**（systemd-networkd + QEMU 用户网络的固定网段兜底），python3 在镜像里就不碰 apt，node 走**官方二进制 tarball**（9p 上 dpkg 解包 nodejs 要十分钟量级，且 `security_model=none` 下 chown 被顶回 EPERM）。全量补齐时这一条失败只如实跳过并继续，显式点名才硬失败——它依赖本机能起 guest）。
- - 解归档的入口程序（清单 `probe` 指到的那个）装完会补**可执行位**：zip 不一定带 unix 权限位，解出来会是不能跑的 0644（例如 `virtiofsd` 的便携包）。
- **完整性**：缓存里那份（复用的、刚下好的）进解包前按台账核对 sha256——自托管 release 资产读清单条目（`sha256:`），
  链接类源读 `links.yaml` 的 `<键>.sha256`（pin 与它守护的 URL 在同一份台账里）；不 pin 的源如实不校验，靠解包/解析成功判。
  不符时**不删缓存**：报出台账值、实际值与来源——删了会在 pin 过期时变成每次重下的死循环。
- 补齐后：主程序**每次判据取用时**自动采用项目内补齐的可执行文件（`qemu_path` / `virtiofsd_path` / `smbd_path` 未登记时；显式登记优先，取到的是拼接产品根后的绝对路径）——
  装完即生效，不需要重启进程：共享后端也**全部在装配期持有**，哪一档在起 guest 那一刻按候选挑（见 [docs/vm/README.md](../vm/README.md) 的「多平台组合」）。
- 进度：作业状态带单调 `seq`；`GET /api/setup/status?since=N&wait=1` 是长轮询（最多 20s），拿到新行即回，前端按 `seq` 增量取。
  **进度只有这一条通道**：`seq` + `lines`（留最近 200 行）——不另存"当前步骤"摘要，界面要看当前步骤就取最后一行。
- **要装什么由后端定**：`status` 的 `pending` 列出还没在场的条目 id；`ensure` 不点名时只补这些（执行核心不跳过在位项，不滤就会把已装的
  重下一遍、重解一遍），都装好了直接回 `{ok,deps:[]}` 且不起作业。显式点名 = 逐条失败硬报（不是全量补齐那种"跳过继续"）。
- 状态归属：清单与台账的唯一真相在仓库根 `dependencies.yaml` / `links.yaml`，产品里**编译期内嵌**；作业状态是**进程内**的（不落盘，进程重启即清空）。
- 呈现层：**Web 的「补齐虚拟机依赖」弹层**——配置面板与创建向导在虚拟机档不可用时给这个动作：先看清单与体积、再确认下载、
  然后按 `seq` 长轮询进度、最后如实说清"哪些立刻生效、哪些要重启"。另有 CLI `--provision`（`src/diagnostics/`，隐藏模式，
  `--ensure[=id,…] --yes` 才真下载；没有界面时用）。路由表见 [docs/presentation/contracts.md](../presentation/contracts.md)。

## 三、依赖图位置（由源码的 ::api 引用推导）

- 经 `::api` 用到：`vm`（`guest-runtime` 的运行期编排要起 guest：`guest_start` / `guest_wait_agent` / `guest_run` / `guest_stop`）；另经 kernel 的机制 `outbound` / `unpack`。
- 谁在用我：`conductor`（`Ops` 的 `provision` 转发面）、`presentation`（setup 路由）、`src/diagnostics/`（`--provision`）。

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