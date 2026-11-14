# 单元地图 · kernel

> 定位：kernel（地基库）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-13｜依据：`src/kernel/api.rs`

> 本文是 **kernel** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> **无领域语义、无领域状态**的机制；形状与别的业务一致（`api` / `ports` / `domain` / `detail`），但它在依赖图的最底层：**不认识任何能力**。
> `Log` / `HostProbe` / `ToolHandler` / `AskUser` / `ProcessRunner` / `SessionHost` / `ProcessTexts` / `FenceHost` 是**全项目共享**的机制端口（R12 的例外）。

| 文件 | 职责 |
| --- | --- |
| `src/kernel.rs` | 地基库的入口与公开面：`contracts` / `api` / `ports` / `domain` / `detail` 五个子模块的形状声明 |
| `src/kernel/api.rs` | 地基库的对外面：共享形状（`SessionId` / `Tier` / `ExecSpec` / `ToolOutcome` / `Ask` / `RunOutcome` / `FenceSpec` / `FencePart` / `FenceBlocked` / 默认预算）与纯机制（`slash` 的对外写法、路径写法 `native` / `is_unc` / `unc_parts`、落点身份 `locus_of` / `place_contains` / `is_network_path`、生成中作业的取消表 `JobRegistry`）的重导出 |
| `src/kernel/ports.rs` | 全项目共享的机制端口：`Log` / `PathCodec` / `ProcessTexts` / `AskUser` / `ToolHandler` / `HostProbe` / `ProcessRunner` / `SessionHost` / `FenceHost` / `FenceGate` / `UserPath` 的 trait 定义与它们的纯数据形态（含 `ShellRun` 的 `gate` 与 shell 收尾文案、用户 PATH 的 `PathValue`） |
| `src/kernel/contracts.rs` | 跨层共享形状的入口：`fence` / `run` / `types` 三个子模块声明 |
| `src/kernel/contracts/types.rs` | 跨层共享的形状（纯数据）：`SessionId` / `Tier` / `ExecSpec` / `ToolOutcome` / `Ask` / `AskOutcome` 与默认预算、能力名文法 |
| `src/kernel/contracts/run.rs` | 宿主执行端口的中性回执 `RunOutcome`（跑没跑、输出是什么；不含业务语义） |
| `src/kernel/contracts/fence.rs` | 围栏描述符（`FenceSpec` / `FenceRunPolicy`）与落点判据（`FencePart` / `FenceBlocked`）：纯数据 |
| `src/kernel/domain/fence.rs` | 沙箱策略对机制的要求与本次模式（`SandboxRequirements` / `SandboxMode`）、按次租约生成：纯机制 |
| `src/kernel/domain/path.rs` | 路径的对外书写形式（门面）：把路径统一写成 `/` 分隔的字符串；拆出 UNC 的 server / share，并给出落点身份 `Locus` 的键；平台实现在 `src/kernel/domain/path/windows.rs` / `src/kernel/domain/path/unix.rs` |
| `src/kernel/domain/path/windows.rs` | `path` 的 Windows 后端：把 `\\?\` / `\\?\UNC\` 扩展前缀还原成普通写法 |
| `src/kernel/domain/path/unix.rs` | `path` 的非 Windows 后端：没有扩展前缀，原样返回 |
| `src/kernel/domain/platform.rs` | 平台事实：可执行文件后缀、PATH 列表分隔符、宿主路径大小写语义、Windows 授权事实、宿主 OS / 架构与卷键 / UNC 纯判断 |
| `src/kernel/domain/jobs.rs` | 生成中作业的取消表（`JobRegistry`）：同一会话每次生成各持一面旗，停止置位全部、摘旗按身份 |
| `src/kernel/domain/receipt.rs` | host/guest 共用的 shell 回执装配（stdout + `[stderr]` 区 + head+tail 截断（按完整行取整）+ 状态行；词表来自 `ProcessTexts`） |
| `src/kernel/detail/file_log.rs` | `Log` 端口的文件实现：每次运行在 `logs/` 下按时间戳建文件并逐行追加（写经 `Mutex` 串行） |
| `src/kernel/detail/host_probe.rs` | `HostProbe` 端口的机制实现（门面）：查环境变量与 `PATH` 里的可执行文件、路径存在性；可执行位与虚拟化事实在 `src/kernel/detail/host_probe/{windows,linux,macos,other}.rs` |
| `src/kernel/detail/host_probe/windows.rs` | `HostProbe` 的 Windows 虚拟化事实：系统目录有 `WinHvPlatform.dll` |
| `src/kernel/detail/host_probe/linux.rs` | `HostProbe` 的 Linux 虚拟化事实：`/dev/kvm` 存在 |
| `src/kernel/detail/host_probe/macos.rs` | `HostProbe` 的 macOS 虚拟化事实：恒可用 |
| `src/kernel/detail/host_probe/other.rs` | `HostProbe` 的兜底虚拟化事实：如实不可用 |
| `src/kernel/detail/fs_meta.rs` | 宿主文件元数据原语（门面）：可执行位、软链、私密权限、占用估算、链接判定、目录刷新与可执行脚本；按平台选中 `src/kernel/detail/fs_meta/{windows,unix,other}.rs` |
| `src/kernel/detail/fs_meta/windows.rs` | `fs_meta` 的 Windows 后端：可执行性按文件、权限位空操作、重解析点判定、占用按长度 |
| `src/kernel/detail/fs_meta/unix.rs` | `fs_meta` 的 Unix 后端：模式位、软链、0600、块占用、0755 脚本 |
| `src/kernel/detail/fs_meta/other.rs` | `fs_meta` 的兜底后端：没有模式位与软链，如实降级 |
| `src/kernel/detail/locus.rs` | 落点身份（门面）：按卷 + 文件 id 判同一物理对象与物理包含；平台实现在 `src/kernel/detail/locus/windows.rs` / `src/kernel/detail/locus/unix.rs` |
| `src/kernel/detail/locus/windows.rs` | `locus` 的 Windows 后端：卷序列号 + 文件 id、UNC 与映射盘判定 |
| `src/kernel/detail/locus/unix.rs` | `locus` 的非 Windows 后端：设备号 + inode、网络路径恒 false |
| `src/kernel/detail/outbound.rs` | 出站 HTTPS 的共享机制（门面）：通用 ureq 代理；TLS 后端装配在 `src/kernel/detail/outbound/windows.rs` / `src/kernel/detail/outbound/unix.rs` |
| `src/kernel/detail/outbound/windows.rs` | `outbound` 的 Windows 后端：native-tls + 系统证书库 |
| `src/kernel/detail/outbound/unix.rs` | `outbound` 的非 Windows 后端：rustls 默认配置 |
| `src/kernel/detail/user_path.rs` | `UserPath` 端口的门面：Windows 实现在 `src/kernel/detail/user_path/windows.rs`，其余平台为空模块 |
| `src/kernel/detail/user_path/windows.rs` | `UserPath` 的 Windows 实现：读写 `HKCU\Environment` 的 `Path`，保留 `REG_SZ` / `REG_EXPAND_SZ` 类型并广播 `WM_SETTINGCHANGE` |
| `src/kernel/detail/unpack.rs` | 归档解包机制（门面）：tar / tar.gz / tar.zst / tar.xz / zip 解到目录，支持 strip 与宽容链接；权限位与软链在 `src/kernel/detail/unpack/{windows,unix}.rs` |
| `src/kernel/detail/unpack/windows.rs` | `unpack` 的 Windows 后端：不设 Unix 权限位；软链按目录 / 文件分开建 |
| `src/kernel/detail/unpack/unix.rs` | `unpack` 的非 Windows 后端：还原归档权限位与 Unix 软链 |
| `src/kernel/detail/process.rs` | 外部进程执行机制（门面）：守门进程、stdin 送参、隐秘字段 env 注入、超时/取消杀树、截断与一次性 shell；平台实现在 `src/kernel/detail/process/{windows,unix}.rs` |
| `src/kernel/detail/process/windows.rs` | `process` 的 Windows 后端：容器围栏准备、Job Object 收树与 PowerShell shell |
| `src/kernel/detail/process/unix.rs` | `process` 的非 Windows 后端：进程组收树与 `sh -c` shell |
| `src/kernel/detail/process/tests.rs` | 进程适配器的真实契约用例：真起进程、真收输出、真判退出码与平台分支（`#[cfg(test)] mod tests;` 指向） |
| `src/kernel/detail/procs/tests.rs` | 进程账与回收的契约用例：登记/注销、存活探测、父死子亡的分平台接线（`#[cfg(test)] mod tests;` 指向） |
| `src/kernel/detail/procs.rs` | 长驻子进程的账（`.home/processes.json`）与回收（门面）：登记、对账与回收流程；平台实现在 `src/kernel/detail/procs/windows.rs` / `src/kernel/detail/procs/unix.rs` |
| `src/kernel/detail/procs/windows.rs` | `procs` 的 Windows 后端：作业对象、创建时间身份、存活探测与 `taskkill` |
| `src/kernel/detail/procs/unix.rs` | `procs` 的非 Windows 后端：进程组、PDEATHSIG、macOS 守门进程、命令行身份与信号终止 |
| `src/kernel/detail/session.rs` | 长驻外部进程机制（门面）：守门进程长跑、按行收发、关闭杀树、进程账登记与注销；平台围栏准备在 `src/kernel/detail/session/{windows,unix}.rs`，实现 `SessionHost` |
| `src/kernel/detail/session/windows.rs` | `session` 的 Windows 后端：常驻服务拉起前的容器围栏准备 |
| `src/kernel/detail/session/unix.rs` | `session` 的非 Windows 后端：没有容器围栏准备 |
| `src/kernel/detail/view_drive.rs` | 视图盘模块门面：只有 Windows 有实现（`src/kernel/detail/view_drive/windows.rs`），其余平台为空模块 |
| `src/kernel/detail/view_drive/windows.rs` | 视图盘的 Windows 实现：产品根映射成实例独占盘符、宿主与视图路径互译、按 pid 登记与退出释放及残留清扫 |
| `src/kernel/detail/view_drive_lifecycle.rs` | 视图盘生命周期门面：分配、清扫、释放与路径编解码；按平台选中 `src/kernel/detail/view_drive_lifecycle/{windows,unix}.rs` |
| `src/kernel/detail/view_drive_lifecycle/windows.rs` | 视图盘生命周期的 Windows 后端：逃生口、真分配与释放 |
| `src/kernel/detail/view_drive_lifecycle/unix.rs` | 视图盘生命周期的非 Windows 后端：不分配、编解码恒 `None` |
| `src/kernel/detail/confine.rs` | 围栏机制门面：平台后端选择与重导出；本机能力与本次实际（`effective_paths`）的判据、自检（`verify`）与 `FenceHost` 释放 |
| `src/kernel/detail/confine/job.rs` | 守门进程的围栏作业：`FenceJob` 的 JSON 形态、启动命令（`launcher`）与执行转交（`run_fenced`）；策略由调用方传入 |
| `src/kernel/detail/confine/admin.rs` | 围栏台账与管理动作的平台分派：授权释放、对账、按条处置、机器级预授与清理 |
| `src/kernel/detail/confine/admin/windows.rs` | `admin` 的 Windows 后端：把动作转交给 `src/kernel/detail/confine/windows/` 的真实实现 |
| `src/kernel/detail/confine/admin/other.rs` | `admin` 的非 Windows 后端：空报告、零计数与如实降级 |
| `src/kernel/detail/confine/ledger.rs` | 围栏授权台账的形状与公开入口：`Owner` / `Ledger` / `ReconcileReport`；平台动作经 `admin.rs` 分派 |
| `src/kernel/detail/confine/paths.rs` | 围栏的路径与环境工具（门面）：PATH / PATHEXT 解析、安装目录上溯、`shell_command` 与 `fence_env`；平台分支在 `src/kernel/detail/confine/paths/{windows,unix}.rs` |
| `src/kernel/detail/confine/paths/windows.rs` | `paths` 的 Windows 后端：真身解析、重解析点判定、node 开关、程序名改写与 `cmd` shell |
| `src/kernel/detail/confine/paths/unix.rs` | `paths` 的非 Windows 后端：执行位、链接与 `sh` shell |
| `src/kernel/detail/confine/prep.rs` | 围栏准备与清理（门面）：`FencePrep` / `FenceStep`、机器级预授与清理；Windows 实现在 `src/kernel/detail/confine/prep/windows.rs` |
| `src/kernel/detail/confine/prep/windows.rs` | `prep` 的 Windows 后端：围栏授权准备、机器级标定与按次回收 |
| `src/kernel/detail/confine/texts.rs` | 围栏边界说明：可达范围摘要（`reachable_roots`）与面向调用方的边界注记（`fence_boundary_note`） |
| `src/kernel/detail/confine/tests.rs` | 围栏机制的单元测试：环境注入、安装目录上溯、Windows 程序名改写、作业 JSON、解释器解析、node realpath、边界说明与租约归属 |
| `src/kernel/detail/confine/linux.rs` | Linux 围栏后端：Landlock 强制可达范围（`spec.rw` 读写、运行基线只读、其余拒绝），seccomp 在 `net=false` 时拒绝非本地 socket；机制不可用时如实降级 |
| `src/kernel/detail/confine/macos.rs` | macOS 围栏后端：生成 seatbelt profile（默认拒绝，再逐条放行运行基线与 `spec.rw`；按 `net` 放行 `network-outbound` / 拒绝）交 `sandbox_init` 装上，装不上如实降级 |
| `src/kernel/detail/confine/other.rs` | 其他平台后端：如实降级——只保证超时与整棵进程树终止，绝不作有文件系统围栏的假装 |
| `src/kernel/detail/confine/windows/` | Windows 围栏后端目录：入口与策略派发在 `windows.rs`（容器 cwd 走 `src/kernel/detail/view_drive.rs` 的视图盘写法，祖先授权不再需要），DACL 授权在 `acl.rs`、机器级一次性预授在 `machine.rs`、授权台账与撤权在 `record/`、容器身份与作业对象在 `container.rs`、真机探针在 `tests.rs` |
| `src/kernel/detail/confine/windows.rs` | Windows 围栏后端入口：按 agent 派生 AppContainer SID、逐条授权可达范围，再用 `STARTUPINFOEX` 在容器与作业对象里起工具（`net` 决定是否授 internetClient） |
| `src/kernel/detail/confine/windows/machine.rs` | Windows 机器级一次性预授：落点清单（产品根 `(OI)(CI)` 读属性、`.tools` / `runtimes` 递归只读）、只读缺项与幂等执行，逐条计时；外部工作区一次性只读的授/撤，以及旧全应用包机器级授权的退役（先授后撤、浅路径先撤、逐条台账事务） |
| `src/kernel/detail/confine/windows/acl.rs` | Windows 安全描述符操作：逐路径改 DACL（授予 / 撤销）、解析 ACE 与 SID、展开泛型掩码、`AccessCheck` 判写 DACL 权限 |
| `src/kernel/detail/confine/windows/ledger_tests.rs` | 台账一族真机用例：快照先落盘、写后核对、按条处置、启动对账与孤儿回收（`#[cfg(test)] mod ledger_tests;` 指向） |
| `src/kernel/detail/confine/windows/record.rs` | Windows 授权台账的入口：`store` / `liveness` / `journal` / `sweep` / `release` / `catalog` / `reconcile` / `paths` / `sid` 九个职责的子模块声明，并向 Windows 后端入口 `windows.rs` 重导出同一个调用面 |
| `src/kernel/detail/confine/windows/record/store.rs` | 台账的盘上格式与互斥：`GrantRecord` / `SnapshotEntry` / `GrantEntry` / `ProfileEntry`、单一锁文件与进程内 `LEDGER_MUTEX`、锁内 `load_record` / `save_record` 与备份轮转 |
| `src/kernel/detail/confine/windows/record/liveness.rs` | 归属者的活性：`current_owner`、进程创建时间与存活探测（`Liveness` / `OwnersState` / `owners_state` / `owner_liveness`） |
| `src/kernel/detail/confine/windows/record/journal.rs` | 授权的建立：写前快照与按条记账（`journal_add_profile` / `journal_claim_grant` / `journal_release_grant` / `journal_add_snapshot` / `grant_one_journaled` / `journal_claim_machine` / `grant_machine_journaled` / `combine`） |
| `src/kernel/detail/confine/windows/record/sweep.rs` | 清扫：容器 profile 与孤儿 ACE（`record_profile` / `sweep_profiles` / `delete_profile` / `sweep_orphan_aces` / `sweep_dir_down`） |
| `src/kernel/detail/confine/windows/record/release.rs` | 撤销与释放：按条撤销（`revoke_one_text`）、围栏释放（`release_fence` / `release_by_lease`，锁内摘归属、锁外撤 ACL 的两阶段）、默认清理（`clean`，保留机器级）与全量清理（`clean_all`） |
| `src/kernel/detail/confine/windows/record/catalog.rs` | 按条处置的读写面：`acl_facts` / `catalog` / `restore_one` / `revoke_grant` / `remove_profile_one` |
| `src/kernel/detail/confine/windows/record/reconcile.rs` | 启动对账：`reconcile` 按归属与活性收掉上一轮运行留下的授权、快照与容器 profile |
| `src/kernel/detail/confine/windows/record/paths.rs` | 台账与围栏的路径判据：产品根（`product_root`）与根内包含（`inside_root`） |
| `src/kernel/detail/confine/windows/record/sid.rs` | SID 的文本与二进制互转（`sid_to_string` / `sid_from_string`） |
| `src/kernel/detail/confine/windows/container.rs` | Windows 容器围栏的启动面：AppContainer 档案与 SID、本产品能力 SID（`solomni.fence.baseline` → `S-1-15-3-*`，profile 与令牌两处都声明）、`net` 对应的 internetClient 能力、UTF-16 转换、system shell 与 kill-on-close 作业对象 |
| `src/kernel/detail/confine/windows/tests.rs` | Windows 围栏真机探针的父模块：共享夹具（`expect_granted` / `dump_aces` / `acl_round_trip` / `discard` / `container_diag` / `node_runs`）与 `container` / `grant` / `workspace` 三个用例族子模块声明 |
| `src/kernel/detail/confine/windows/tests/container.rs` | 容器内真机往返用例族：叶子读得到写得到、父目录内容列不到、模块只读、绝对路径程序、按次临时目录与并发隔离 |
| `src/kernel/detail/confine/windows/tests/grant.rs` | 围栏授权用例族：落点清单与 STAT 祖先、权限位覆盖、写后核对、台账归属与撤销、失败回滚 |
| `src/kernel/detail/confine/windows/tests/workspace.rs` | 视图盘与外部工作区用例族：容器 cwd 落在映射盘符、机器级一次性只读首写与复用、显式 ensure 与撤权 |
