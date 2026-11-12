# 单元地图 · provision

> 定位：provision（依赖补齐）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/provision/api.rs`

> 本文是 **provision** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 依赖补齐能力：清单 / 台账解析、本机状态、下载缓存、解包落盘、后台作业，以及虚拟根的构造（`guest-rootfs` / `guest-disk`）与运行期编排（`guest-runtime`）。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/provision.rs` | 能力入口：模块与公开面（`ProvisionService`），以及组合根用的装配工厂 `assemble` / `assemble_with_vm` |
| `src/capabilities/provision/api.rs` | 入站契约：`Provision` trait + `DepStatus`（含 `stale`：探针在场但装的不是当前清单那版）/ `JobState` DTO + `NoProvision` |
| `src/capabilities/provision/domain.rs` | 清单语义：`dependencies.yaml` / `links.yaml` 解析、按本机筛选、安装身份、pin 与 guest 画像 `GuestProfile` |
| `src/capabilities/provision/ports.rs` | 出站端口：`Fetcher`（取数）/ `HostFs`（宿主落点）/ `Ledger`（台账与在场判定）/ `Installers`（按 kind 落盘）/ `GuestImage`（rootfs 镜像构造）/ `Procs`（进程台账） |
| `src/capabilities/provision/service.rs` | 编排：状态与台账快照、依赖选择（`split_ensure_spec` / `pending_of_kinds`）、每条的装 / 验 / 记、作业与取消；只经端口触达 IO |
| `src/capabilities/provision/guest.rs` | 虚拟根的运行期编排（`guest-runtime`）：Windows 走 cloud-init 种子首启装 agent、Linux 跳过首启；维护启动经 QGA 装 python3、C-C++ 与固定版本 Node，逐个自证后落 `.qga-ready` / `.runtimes-ready` 并改写 `guest.yaml` 的 `clients` / `runtimes` |
| `src/capabilities/provision/detail.rs` | 适配器入口：登记六个实现模块 |
| `src/capabilities/provision/detail/fetch.rs` | `HttpFetch`：ureq 取文本、sha256、选源测速与断点续传 |
| `src/capabilities/provision/detail/host.rs` | `RealHost`：存在性、建目录、原子写、删除、执行位；以及 `symlink` / `run_tool` / `write_exe` 等宿主原语 |
| `src/capabilities/provision/detail/ledger.rs` | `FileLedger`：`.vm/state/deps.json` 读写、探针在场判定、装好登记与清旧、`active.json` 的工具链登记 |
| `src/capabilities/provision/detail/install.rs` | `HostInstallers`：归档解包、单文件复制、AppImage 自解包、Zig 包装脚本、guest 磁盘与 `guest.yaml` 画像 |
| `src/capabilities/provision/detail/guest_image.rs` | `Ext4Images` 适配器：`install_guest_rootfs` 的多步构造（镜像 tar → rootfs 目录 → 内核 → initrd → agent → ext4）、基线打包、`guest.yaml` 画像补全 |
| `src/capabilities/provision/detail/guest_image/deb.rs` | deb 与内核 / agent 的选型、解包与落位：分区定位、版本比较、`ar` 解包、ABI 判定、`vmlinuz` / `initrd.img` 软链 |
| `src/capabilities/provision/detail/guest_image/initrd.rs` | initrd 配方与生成：`GuestFamily` 表、initramfs-tools 配置与脚本、模块去重合并、`unshare` + `chroot` 执行、失败输出尾 |
| `src/capabilities/provision/detail/guest_image/image.rs` | ext4 属性与调试工具：`GuestAttr` 回填、`debugfs` 命令拼装、目录占用统计、保留清单之外的清理 |
| `src/capabilities/provision/detail/procs.rs` | `BookProcs`：进程台账条目映射与 `guest` 是否在跑 |
