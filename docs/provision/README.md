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
- 安装方式（清单 `kind`）：`archive`（默认，解归档）/ `appimage`（自带 `--appimage-extract`，不依赖 FUSE）/ `file`（单文件直下，按 probe 的文件名落盘）/ `guest-disk`（单文件 + `guest.yaml` 画像）/ `guest-rootfs`（Linux guest 虚拟根多步构造：cloud 镜像 → rootfs + 通用内核 + 9p initrd + qemu-guest-agent，并写一份基础 `guest.yaml` 画像；调 `tar` / `sfdisk` / `dd` / `debugfs` / `ar` / `unshare`，只写项目内）/ `guest-runtime`（虚拟根的运行期编排：经 `vm::api` 起一次 guest，先整份刷新 apt lists，再装运行包并**逐个自证**（`python -V` / `node --version`），只把自证过的能力写进 `guest.yaml` 的 `runtimes`，并落 `.runtimes-ready` 标记；磁盘根还要首启经 cloud-init 种子装 `qemu-guest-agent`、装 `cifs-utils` 并把客户端声明写成 cifs，目录根（Linux）的 agent 由 `guest-rootfs` 装好、客户端保持 9p/virtiofs；只写项目内。全量补齐时这一条失败只如实跳过并继续，显式点名才硬失败——它依赖本机能起 guest）。
- 补齐后：主程序**启动时**自动采用项目内补齐的可执行文件（`qemu_path` / `virtiofsd_path` / `smbd_path` 未登记时；显式登记优先）；同进程内新装的包下次启动生效。
- 进度：作业状态带单调 `seq`；`GET /api/setup/status?since=N&wait=1` 是长轮询（最多 20s），拿到新行即回，前端按 `seq` 增量取。
- 状态归属：清单与台账的唯一真相在仓库根 `dependencies.yaml` / `links.yaml`，产品里**编译期内嵌**；作业状态是**进程内**的（不落盘，进程重启即清空）。
- 呈现层：CLI `--provision`（`src/diagnostics/`，`--ensure[=id,…] --yes` 才真下载）+ Web `GET /api/setup/status` / `POST /api/setup/ensure` / `DELETE /api/setup/job`（路由表见 [docs/presentation/contracts.md](../presentation/contracts.md)）。

## 三、依赖图位置（由源码的 ::api 引用推导）

- 经 `::api` 用到：`vm`（`guest-runtime` 的运行期编排要起 guest：`guest_start` / `guest_wait_agent` / `guest_run` / `guest_stop`）；另经 kernel 的机制 `outbound` / `unpack`。
- 谁在用我：`conductor`（`Ops` 的 `provision` 转发面）、`presentation`（setup 路由）、`src/diagnostics/`（`--provision`）。

## 四、改动本单元时必须同步

- 改清单字段或新增依赖 → `dependencies.yaml` 与 `dependencies/THIRD-PARTY-NOTICES.md`（自托管包的许可证与对应源码）；需要新下载源时改 `links.yaml`。
- 改入站契约或路由 → `docs/presentation/contracts.md` 的 ROUTES 表与 `src/tests/routes.rs`（契约测试机器比对）。
- 改下载 / 解包机制 → kernel 的 `outbound` / `unpack` 单元地图。
- 改 guest 运行期编排 → [docs/vm/README.md](../vm/README.md) 与 vm 的 `GuestSpec.seed` / `guest_wait_agent` 契约。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |