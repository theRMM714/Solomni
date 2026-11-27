# 单元地图 · kernel

> 定位：kernel（地基库）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/kernel/api.rs`、`docs/kernel/mechanisms.yaml`

> 本文是 **kernel** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> **无领域语义、无领域状态**的机制；形状与别的业务一致（`api` / `ports` / `domain` / `detail`），但它在依赖图的最底层：**不认识任何单元**。
> `Log` / `HostProbe` / `ToolHandler` / `AskUser` / `ProcessRunner` / `SessionHost` / `FenceHost` 是**全项目共享**的机制端口（R12 的例外）。

| 文件 | 职责 |
| --- | --- |
| `src/kernel.rs` | 地基库的入口与公开面：`contracts` / `api` / `ports` / `domain` / `detail` 五个子模块的形状声明 |
| `src/kernel/api.rs` | 地基库对外面的入口：只声明 `api/` 下的面模块。面按**对外概念**切（`fence` / `note` / `run` / `types` / `clock` / `fs_meta` / `host_probe` / `i18n` / `locus` / `process` / `procs` / `lock` / `view_drive` / `actor` / `jobs` / `layout` / `name` / `net` / `path` / `platform` / `hash` / `budget` / `path_bridge`），每个面只做重导出、不定义条目；能力与装配层经 `api::<面>` 取用，内核内部直取所属机制（见 [README](README.md) 的「六、对外面」） |
| `src/kernel/ports.rs` | 全项目共享的机制端口：`Log` / `PathCodec` / `AskUser` / `ToolHandler` / `HostProbe` / `ProcessRunner` / `SessionHost` / `FenceHost` / `FenceGate` / `UserPath` 的 trait 定义与它们的纯数据形态（含 `ShellRun` 的 `gate`、用户 PATH 的 `PathValue`） |
| `src/kernel/contracts.rs` | 跨层共享形状的入口：`fence` / `note` / `run` / `types` 四个子模块声明 |
| `src/kernel/contracts/types.rs` | 跨层共享的端口载荷（纯数据）：`SessionId` / `ToolOutcome` / `Ask` / `AskOutcome` 与默认预算、能力名文法（执行档位与作业进度归 `exec_model`，不在 kernel） |
| `src/kernel/contracts/note.rs` | 回执说明的契约：`Note`（稳定 ASCII 码 + 类型化参数）、`Severity`（`info` / `warning` / `error` 与 `Ord` 优先级）与码表 `codes`（含导出给界面遍历的 `ALL`） |
| `src/kernel/contracts/run.rs` | 宿主执行端口的中性回执 `RunOutcome`（跑没跑、轻重、输出、说明；不含业务语义与文案） |
| `src/kernel/contracts/fence.rs` | 围栏**意图**：`FencePlan`（`home` / `cwd` / `grants` / `net`）、`Grant`（`path` + `Access` + `Reach`）、`NetPolicy`、`FenceRunPolicy` 与 `FenceBlocked`：纯数据，不含任何平台词汇 |
| `src/kernel/domain/actor.rs` | 命令队列（`Actor<S>`）：把一份核心状态搬到它自己的线程上，命令按到达顺序逐条执行，一条命令 panic 不带走整个核心；线程退出与 panic 都落日志。不含命令的业务语义 |
| `src/kernel/domain/hash.rs` | 内容指纹 SHA-256：增量实现（`Sha256` / `content_hash` / `is_content_hash`），共享区内容寻址与运行包/素材指纹的唯一键（纯逻辑，对外经 `api::hash`） |
| `src/kernel/domain/budget.rs` | 工具回执预算：`ResultBudget`（`full` / `for_batch` / `pressure` / `exhausted`）、`Continuation`（`Command` / `Read`）、`cap_result_with` 与 `MAX_RESULT_CHARS` / `MIN_RESULT_CHARS` 等常量；按行边界截断、留一行预算说明（纯逻辑，对外经 `api::budget`） |
| `src/kernel/domain/path_bridge.rs` | 路径方言桥：`PathDialect`（`Posix` / `Windows`）与 `PathBridge`（`for_session` / `to_guest` / `round_trips`），把宿主路径与客机路径互译（POSIX 镜像 `/solomni-host`、Windows 镜像 `C:\solomni-host` 与 UNC 段）；虚拟机的路径书写与宿主的视图路径共用这一份（纯逻辑，对外经 `api::path_bridge`） |
| `src/kernel/domain/fence.rs` | 沙箱策略对机制的要求与本次模式（`SandboxRequirements` / `SandboxMode`）、按次租约生成：纯机制 |
| `src/kernel/domain/path.rs` | 路径的对外书写形式（门面）：把路径统一写成 `/` 分隔的字符串；拆出 UNC 的 server / share，并给出落点身份 `Locus` 的键；平台实现在 `src/kernel/domain/path/windows.rs` / `src/kernel/domain/path/unix.rs` |
| `src/kernel/domain/path/windows.rs` | `path` 的 Windows 后端：把 `\\?\` / `\\?\UNC\` 扩展前缀还原成普通写法 |
| `src/kernel/domain/path/unix.rs` | `path` 的非 Windows 后端：没有扩展前缀，原样返回 |
| `src/kernel/domain/platform.rs` | 平台事实：可执行文件后缀、PATH 列表分隔符、宿主路径大小写语义、Windows 授权事实、宿主 OS / 架构与卷键 / UNC 纯判断 |
| `src/kernel/domain/jobs.rs` | 生成中作业的取消表（`JobRegistry`）：同一会话每次生成各持一面旗，停止置位全部、摘旗按身份 |
| `src/kernel/domain/name.rs` | 会成为目录名的名字的两条纯规则：`validate_dir_name`（非空、无首尾空白、不含 Windows 非法字符与控制字符、不是设备保留名，主语只进报错）与 `dedup_suffix`（占用时依次试 `-2` / `-3`） |
| `src/kernel/domain/layout.rs` | 产品根的布局名字（`.home` / `.vm` / `runtimes` / `guest.yaml`）与由根派生的四处路径：`home` / `vm` / `runtimes` / `guest_spec`，调用点不再各自拼字面量 |
| `src/kernel/domain/receipt.rs` | host/guest 共用的 shell 回执装配：`ShellReceipt { payload, notes }`——stdout 与 `[stderr]` 区进正文、结束状态（退出码 / 信号 / 超时 / 用户停止）成说明；只装配不截断，预算在 `src/kernel/domain/budget.rs` |
| `src/kernel/detail/clock.rs` | 时间机制（门面）：`Local`（本机时区的年月日时分秒毫秒）与此刻的 `local_now`、到分钟流水戳 `local_stamp`（`YYYYMMDD-HHMM`）、记时刻用的 `now_secs` / `now_ms`（自 Unix 纪元）；按平台选中 `src/kernel/detail/clock/{windows,unix}.rs` |
| `src/kernel/detail/clock/windows.rs` | `clock` 的 Windows 后端：`GetLocalTime` 取本机时区的民用时间 |
| `src/kernel/detail/clock/unix.rs` | `clock` 的非 Windows 后端：`localtime_r` 转本地民用时间，时区坏掉时退到 `gmtime_r` 的 UTC 值 |
| `src/kernel/detail/file_log.rs` | `Log` 端口的文件实现：每次运行在 `data/logs/` 下按本地时间戳建文件并逐行追加（写经 `Mutex` 串行），时间字段取自 `clock` |
| `src/kernel/detail/host_probe.rs` | `HostProbe` 端口的机制实现（门面）：查环境变量与 `PATH` 里的可执行文件、路径存在性；可执行位与虚拟化事实在 `src/kernel/detail/host_probe/{windows,linux,macos,other}.rs` |
| `src/kernel/detail/host_probe/windows.rs` | `HostProbe` 的 Windows 虚拟化事实：系统目录有 `WinHvPlatform.dll` |
| `src/kernel/detail/host_probe/linux.rs` | `HostProbe` 的 Linux 虚拟化事实：`/dev/kvm` 存在 |
| `src/kernel/detail/host_probe/macos.rs` | `HostProbe` 的 macOS 虚拟化事实：恒可用 |
| `src/kernel/detail/host_probe/other.rs` | `HostProbe` 的兜底虚拟化事实：如实不可用 |
| `src/kernel/detail/fs_meta.rs` | 宿主文件元数据原语（门面）：可执行位、软链、私密权限、占用估算、链接判定、目录刷新与可执行脚本；按平台选中 `src/kernel/detail/fs_meta/{windows,unix,other}.rs` |
| `src/kernel/detail/fs_meta/windows.rs` | `fs_meta` 的 Windows 后端：可执行性按文件、权限位空操作、重解析点判定、占用按长度 |
| `src/kernel/detail/fs_meta/unix.rs` | `fs_meta` 的 Unix 后端：模式位、软链、0600、块占用、0755 脚本 |
| `src/kernel/detail/fs_meta/other.rs` | `fs_meta` 的兜底后端：没有模式位与软链，如实降级 |
| `src/kernel/detail/lock.rs` | 跨进程文件锁的协议（门面）：`lock_within` 打开锁文件、25ms 轮询 `try_lock`、有界等待；`try_lock` 非阻塞拿锁（有人持着给 `None`）；`held` 只问不拿（不建锁文件）。超时与打开失败都报原因 |
| `src/kernel/detail/lock/tests.rs` | 文件锁协议的契约用例：真文件真句柄，拿到后第二次在等待上限内超时、放掉后可再拿（`#[cfg(test)] mod tests;` 指向） |
| `src/kernel/detail/locus.rs` | 落点身份（门面）：按卷 + 文件 id 判同一物理对象与物理包含；平台实现在 `src/kernel/detail/locus/windows.rs` / `src/kernel/detail/locus/unix.rs` |
| `src/kernel/detail/locus/windows.rs` | `locus` 的 Windows 后端：卷序列号 + 文件 id、UNC 与映射盘判定 |
| `src/kernel/detail/locus/unix.rs` | `locus` 的非 Windows 后端：设备号 + inode、网络路径恒 false |
| `src/kernel/detail/io.rs` | IO 面（门面）：唯一直接碰 OS / 网络 / 进程 / 时钟的一层，声明 IO 子模块（`env`：进程环境；`net`：出站网络与入站回环） |
| `src/kernel/detail/io/net.rs` | 网络原语（门面）：声明 `backend` / `loopback` / `outbound` / `tcp`；出站让 ureq 与 reqwest 两条通道共用一份平台 TLS 决策，入站与回环 TCP 也只在这里落地 |
| `src/kernel/detail/io/net/backend.rs` | 平台 TLS 装配门面：三个出口（ureq 的 TLS 配置、reqwest 构建器注入、后端名称），按平台选中 `backend/{windows,unix}.rs` |
| `src/kernel/detail/io/net/backend/windows.rs` | `backend` 的 Windows 后端：ureq 显式 native-tls + 系统证书库；reqwest 的 native-tls 由依赖特性定死，构建器原样返回 |
| `src/kernel/detail/io/net/backend/unix.rs` | `backend` 的非 Windows 后端：rustls + ring + 系统证书验证器；ureq 用 rustls 默认 |
| `src/kernel/detail/io/net/outbound.rs` | ureq 代理构建：连接 / 总超时预算，4xx/5xx 当**数据**不当错误（状态由调用方判） |
| `src/kernel/detail/io/net/loopback.rs` | 入站回环 HTTP 服务：只绑 127.0.0.1、请求体有界（坏 body 直接 400，handler 见不到）、每请求一线程、`stop` 能收回线程；交出 `Call` / `Answer` 两个中性形状 |
| `src/kernel/detail/io/net/tcp.rs` | 回环 TCP 原语：`reserve_port`（先占后放，只压小碰撞面）、`dial`（唯一写死回环客户端地址处）、`await_loopback`（带「对端已死」回调的就绪等待） |
| `src/kernel/detail/io/env.rs` | 进程环境的唯一读取面：`var` / `os` 两个原语 + 具名宿主事实（PATH、PATHEXT、SystemRoot、LOCALAPPDATA、可执行文件、临时目录） |
| `src/kernel/detail/user_path.rs` | `UserPath` 端口的门面：Windows 实现在 `src/kernel/detail/user_path/windows.rs`，其余平台为空模块 |
| `src/kernel/detail/user_path/windows.rs` | `UserPath` 的 Windows 实现：读写 `HKCU\Environment` 的 `Path`，保留 `REG_SZ` / `REG_EXPAND_SZ` 类型并广播 `WM_SETTINGCHANGE` |
| `src/kernel/detail/unpack.rs` | 归档解包机制（门面）：tar / tar.gz / tar.zst / tar.xz / zip 解到目录，支持 strip 与宽容链接；权限位与软链在 `src/kernel/detail/unpack/{windows,unix}.rs` |
| `src/kernel/detail/unpack/windows.rs` | `unpack` 的 Windows 后端：不设 Unix 权限位；软链按目录 / 文件分开建 |
| `src/kernel/detail/unpack/unix.rs` | `unpack` 的非 Windows 后端：还原归档权限位与 Unix 软链 |
| `src/kernel/detail/process.rs` | 外部进程执行机制（门面）：守门进程、stdin 送参、隐秘字段 env 注入、超时/取消杀树与一次性 shell；平台实现在 `src/kernel/detail/process/{windows,unix}.rs` |
| `src/kernel/detail/process/windows.rs` | `process` 的 Windows 后端：容器围栏准备、Job Object 收树与 PowerShell shell |
| `src/kernel/detail/process/windows/tests.rs` | Windows 解释器解析的纯用例：真 pwsh / 应用执行别名 / 缺失三态，以及显式覆盖优先（`#[cfg(test)] mod tests;` 指向） |
| `src/kernel/detail/process/unix.rs` | `process` 的非 Windows 后端：进程组收树与 `sh -c` shell |
| `src/kernel/detail/process/tests.rs` | 进程适配器的真实契约用例：真起进程、真收输出、真判退出码与平台分支（`#[cfg(test)] mod tests;` 指向） |
| `src/kernel/detail/i18n.rs` | 回执说明的渲染与词表：语言协商（设置 → `SOLOMNI_LANG` → 源语言）、`locales/zh-CN.ftl` 的 Fluent 渲染、把整份词表导出给界面（键仍是稳定码） |
| `src/kernel/detail/procs/tests.rs` | 进程账与回收的契约用例：登记/注销、存活探测、父死子亡的分平台接线（`#[cfg(test)] mod tests;` 指向） |
| `src/kernel/detail/procs.rs` | 长驻子进程的账（`data/state/processes.json`）与回收（门面）：登记、对账与回收流程，以及施工面的底层零件（`guard` / `spawn_raw` / `attach`，只给同目录的 `spawn.rs` 用）；平台实现在 `src/kernel/detail/procs/windows.rs` / `src/kernel/detail/procs/unix.rs` |
| `src/kernel/detail/procs/spawn.rs` | 长驻子进程的唯一施工面：`Spawn`（命令行 / 三路流 / 账上身份 / stderr 去处）与 `spawn`——一次办完进程组长、父死子亡、长命线程、stderr 常驻抽干与入账，交出 `Daemon`；一行 stderr 的 UTF-8 宽容解码也在这里 |
| `src/kernel/detail/procs/daemon.rs` | 长驻子进程的句柄（`Daemon`）：问一次退没退 / 有界等退出 / stderr 尾行 / 取走两条管道 / 收场（先礼后兵，按进程组或整棵树），丢掉即收掉；抽干线程写的有界尾行 `Tail` 也在这里 |
| `src/kernel/detail/procs/windows.rs` | `procs` 的 Windows 后端：作业对象、创建时间身份、存活探测与 `taskkill` |
| `src/kernel/detail/procs/unix.rs` | `procs` 的非 Windows 后端：进程组、PDEATHSIG、macOS 守门进程、命令行身份与信号终止 |
| `src/kernel/detail/session.rs` | 长驻外部进程机制（门面）：守门进程长跑、按行收发、关闭杀树、进程账登记与注销；平台围栏准备在 `src/kernel/detail/session/{windows,unix}.rs`，实现 `SessionHost` |
| `src/kernel/detail/session/windows.rs` | `session` 的 Windows 后端：常驻服务拉起前的容器围栏准备 |
| `src/kernel/detail/session/unix.rs` | `session` 的非 Windows 后端：没有容器围栏准备 |
| `src/kernel/detail/view_drive.rs` | 视图盘模块门面：只有 Windows 有实现（`src/kernel/detail/view_drive/windows.rs`），其余平台为空模块 |
| `src/kernel/detail/view_drive/windows.rs` | 视图盘的 Windows 实现：产品根映射成实例独占盘符、宿主与视图路径互译、按「pid + 进程创建时刻」登记归属与退出释放；归属已死（pid 被复用 / 创建时刻对不上 / 记录缺归属证据）与失效登记（映射已不在 / 目标不匹配 / 无人使用）在启动清扫与 `--fence-clean` 里摘掉，条目清空即删记录文件 |
| `src/kernel/detail/view_drive_lifecycle.rs` | 视图盘生命周期门面：分配、清扫、释放与路径编解码；按平台选中 `src/kernel/detail/view_drive_lifecycle/{windows,unix}.rs` |
| `src/kernel/detail/view_drive_lifecycle/windows.rs` | 视图盘生命周期的 Windows 后端：逃生口、真分配与释放、控制台终止（Ctrl+C / 关窗 / 注销 / 关机）时的退出清理 |
| `src/kernel/detail/view_drive_lifecycle/unix.rs` | 视图盘生命周期的非 Windows 后端：不分配、编解码恒 `None` |
| `src/kernel/detail/confine.rs` | 围栏**实现**门面：`trait Confine`（`capability` / `verify` / `effective_paths` / `local_relay` / `run`）与平台后端选择（Windows 另有自己的 `prepare_fence` 授权入口，不属于跨平台契约）——上层只认计划，平台差异全在四个实现里；另含 `FenceHost` 释放适配（台账动作在 `src/kernel/detail/ledger.rs`） |
| `src/kernel/detail/confine/job.rs` | 守门进程的围栏作业：`FenceJob` 的 JSON 形态、启动命令（`launcher`）与执行转交（`run_fenced`）；策略由调用方传入；`NetPolicy::Local` 在 `local_relay()` 为假的平台上在这里拒绝执行 |
| `src/kernel/detail/confine/prep.rs` | 围栏准备与机器级预授（门面）：`FencePrep` / `FenceStep` 的成败记录与步骤计时；机器级预授按平台经 `src/kernel/detail/confine/prep/windows.rs` / `src/kernel/detail/confine/prep/other.rs` 转交 |
| `src/kernel/detail/confine/prep/windows.rs` | `prep` 的 Windows 后端：围栏授权准备、按次回收与机器级预授的转交 |
| `src/kernel/detail/confine/prep/other.rs` | `prep` 的非 Windows 后端：没有机器级预授时如实返回未支持，退役为空操作 |
| `src/kernel/detail/confine/tests.rs` | 围栏机制的单元测试：环境注入、安装目录上溯、Windows 程序名改写、作业 JSON、解释器解析、node realpath 与租约归属 |
| `src/kernel/detail/confine/linux.rs` | Linux 围栏后端：Landlock 按计划的授予强制可达范围（`Write` 可写、`Read` 只读、其余拒绝），seccomp 在 `net` 关时拒绝非本地 socket；机制不可用时如实降级 |
| `src/kernel/detail/confine/macos.rs` | macOS 围栏后端：生成 seatbelt profile（默认拒绝，再按计划的授予逐条放行运行基线与可写根；按 `net` 放行 `network-outbound` / 拒绝）交 `sandbox_init` 装上，装不上如实降级 |
| `src/kernel/detail/confine/other.rs` | 其他平台后端：如实降级——只保证超时与整棵进程树终止，绝不作有文件系统围栏的假装 |
| `src/kernel/detail/confine/windows.rs` | Windows 围栏后端入口：按 agent 派生 AppContainer SID 并编排授权（逐条写 ACL 在 `acl`、容器内启动在 `container`、写台账在 `ledger`），产出 `FencePrep` 与围栏执行 |
| `src/kernel/detail/confine/windows/machine.rs` | Windows 机器级一次性预授：落点清单（产品根 `(OI)(CI)` 读属性、`.tools` / `runtimes` 递归只读）、只读缺项与幂等执行，逐条计时；旧全应用包机器级授权的退役（先授后撤、浅路径先撤、逐条台账事务） |
| `src/kernel/detail/confine/windows/tests.rs` | Windows 围栏真机探针的父模块：共享夹具（`expect_granted` / `dump_aces` / `acl_round_trip` / `discard` / `container_diag` / `node_runs`）与 `container` / `grant` / `workspace` 三个用例族子模块声明 |
| `src/kernel/detail/confine/windows/tests/container.rs` | 容器内真机往返用例族：叶子读得到写得到、父目录内容列不到、模块只读、绝对路径程序、按次临时目录与并发隔离 |
| `src/kernel/detail/confine/windows/tests/grant.rs` | 围栏授权用例族：落点清单与 STAT 祖先、权限位覆盖、写后核对、台账归属与撤销、失败回滚、完整性标签挡写 |
| `src/kernel/detail/confine/windows/tests/workspace.rs` | 视图盘用例：容器 cwd 落在映射盘符 |
| `src/kernel/detail/container.rs` | 容器身份与容器内启动的门面：只有 Windows 有实现（`src/kernel/detail/container/windows.rs`）；其余平台走 `src/kernel/detail/container/other.rs` 的如实拒绝 |
| `src/kernel/detail/container/other.rs` | 容器能力的非 Windows 后端：回环中继入口如实拒绝（`FENCE_FAILED`），不假装有容器 |
| `src/kernel/detail/container/windows/relay.rs` | Windows 回环中继：容器内 bind `127.0.0.2:<端口>`、宿主按端口清单 dial `127.0.0.1:<端口>`，两侧用继承的匿名管道传帧；端口白名单与「产品自己的端口」都在宿主侧判（后者按「监听者 pid 落在本产品进程树，或它跑的就是本产品这个可执行文件」判，不靠端口表） |
| `src/kernel/detail/container/windows.rs` | Windows 容器围栏的启动面：AppContainer 档案与 SID、本产品能力 SID（`solomni.fence.baseline` → `S-1-15-3-*`，profile 与令牌两处都声明）、`net` 对应的 internetClient 能力、UTF-16 转换、system shell 与 kill-on-close 作业对象、容器内启动（`container_command` / `spawn_in_container` / `wait_and_reap` 三个零件，回环中继也复用） |
| `src/kernel/detail/launch.rs` | 命令启动形状（门面）：PATH / PATHEXT 解析、安装目录上溯、`shell_command` 与 `fence_env`；平台分支在 `src/kernel/detail/launch/windows.rs` / `src/kernel/detail/launch/unix.rs` |
| `src/kernel/detail/launch/windows.rs` | `launch` 的 Windows 后端：真身解析、重解析点判定、node 开关、程序名改写与 `cmd` shell |
| `src/kernel/detail/launch/unix.rs` | `launch` 的非 Windows 后端：执行位、链接与 `sh` shell |
| `src/kernel/detail/ledger.rs` | 围栏授权台账的形状与公开入口：`Owner` / `Ledger` / `ReconcileReport`；台账动作经 `src/kernel/detail/ledger/admin.rs` 分派，盘上格式在 `src/kernel/detail/ledger/windows.rs` |
| `src/kernel/detail/ledger/admin.rs` | 台账动作的平台分派：授权释放、对账、按条处置与全量回收（机器级预授在 `confine` 的 `prep.rs`） |
| `src/kernel/detail/ledger/admin/windows.rs` | `admin` 的 Windows 后端：把台账动作转交给 `src/kernel/detail/ledger/windows/` 与容器身份的真实实现 |
| `src/kernel/detail/ledger/admin/other.rs` | `admin` 的非 Windows 后端：空报告、零计数与如实降级 |
| `src/kernel/detail/ledger/windows.rs` | Windows 授权台账的入口：`store` / `liveness` / `journal` / `sweep` / `release` / `catalog` / `reconcile` / `paths` 八个职责的子模块声明，并向台账门面重导出同一个调用面 |
| `src/kernel/detail/ledger/windows/store.rs` | 台账的盘上格式与互斥：`GrantRecord` / `SnapshotEntry` / `GrantEntry` / `ProfileEntry`、单一锁文件与进程内 `LEDGER_MUTEX`、锁内 `load_record` / `save_record` 与备份轮转 |
| `src/kernel/detail/ledger/windows/liveness.rs` | 归属者的活性：`current_owner`、进程创建时间与存活探测（`Liveness` / `OwnersState` / `owners_state` / `owner_liveness`） |
| `src/kernel/detail/ledger/windows/journal.rs` | 授权的建立：写前快照与按条记账（`journal_add_profile` / `journal_claim_grant` / `journal_release_grant` / `journal_add_snapshot` / `grant_one_journaled` / `journal_claim_machine` / `grant_machine_journaled` / `combine`） |
| `src/kernel/detail/ledger/windows/sweep.rs` | 清扫：容器 profile 与孤儿 ACE（`record_profile` / `sweep_profiles` / `delete_profile` / `sweep_orphan_aces` / `sweep_dir_down`） |
| `src/kernel/detail/ledger/windows/release.rs` | 撤销与释放：按条撤销（`revoke_one_text`）、围栏释放（`release_fence` / `release_by_lease`，锁内摘归属、锁外撤 ACL 的两阶段）、默认清理（`clean`，保留机器级）与全量清理（`clean_all`） |
| `src/kernel/detail/ledger/windows/catalog.rs` | 按条处置的读写面：`acl_facts` / `catalog` / `restore_one` / `revoke_grant` / `remove_profile_one` |
| `src/kernel/detail/ledger/windows/reconcile.rs` | 启动对账：`reconcile` 按归属与活性收掉上一轮运行留下的授权、快照与容器 profile |
| `src/kernel/detail/ledger/windows/paths.rs` | 台账与围栏的路径判据：产品根（`product_root`）与根内包含（`inside_root`） |
| `src/kernel/detail/ledger/windows/ledger_tests.rs` | 台账一族真机用例：快照先落盘、写后核对、按条处置、启动对账与孤儿回收（`#[cfg(test)] mod ledger_tests;` 指向；夹具复用 `confine` 的探针助手） |
| `src/kernel/detail/acl.rs` | ACL 机制门面：按平台选中 `src/kernel/detail/acl/windows.rs` 或 `src/kernel/detail/acl/other.rs`；非 Windows 只答解释器目录可用 |
| `src/kernel/detail/acl/windows.rs` | `acl` 的 Windows 后端：`ace` / `grant` / `label` / `part` / `target` / `scan` / `verify` / `access` / `sid` 子模块声明与统一重导出；权限位常量、`TreeSetNamedSecurityInfoW` 声明与解释器目录基线判据也在这里 |
| `src/kernel/detail/acl/other.rs` | `acl` 的非 Windows 后端：没有 DACL 可写，`interpreter_dir_usable` 恒放行 |
| `src/kernel/detail/acl/windows/ace.rs` | ACE 布局与身份：类型常量、`AceParts` / `AceView` / `DaclScan` / `AceEntry`、`ace_parts` 偏移算法、GUID 文本、`expand_generics` / `rights_covered` |
| `src/kernel/detail/acl/windows/grant.rs` | 授权写入与撤销：`grant_one` / `grant_one_object_ace` / `write_ace` / `revoke_one` |
| `src/kernel/detail/acl/windows/label.rs` | 强制完整性标签：`integrity_label` 读数（`LABEL_SECURITY_INFORMATION`，免提权）、`parse_label_sacl` 纯解析、`blocking_integrity_rid` 挡写判据与 `integrity_name` 级别名 |
| `src/kernel/detail/acl/windows/part.rs` | 落点环节：`FencePart`（必要 / 可选判据的唯一处）、环节名与补法，以及环节 → `FenceBlocked` 的翻译 |
| `src/kernel/detail/acl/windows/target.rs` | 授权落点清单：`GrantTarget` / `write_part` / `grant_targets`、可信执行基座与系统基线 SID（解释器目录由调用方经 `launch` 解析后传入） |
| `src/kernel/detail/acl/windows/scan.rs` | 读 DACL 与还原：`acl_scan` / `acl_entries` / `permission_entries` / `lost_entries` / `sd_bytes` / `restore_sd` / `orphan_package_aces` |
| `src/kernel/detail/acl/windows/verify.rs` | 写后核对与在场判断：`grant_verified` / `restore_after_failed_write` / `has_*_ace_for` / `inheritable_flags_ok` |
| `src/kernel/detail/acl/windows/access.rs` | 写 DACL 的资格预检：`owner_is_current_user` / `can_write_dacl` / `current_user_sid` 与模拟令牌、句柄与内存收尾 |
| `src/kernel/detail/acl/windows/sid.rs` | SID 的文本与二进制互转（`sid_to_string` / `sid_from_string`）；ACL 与台账共用同一份解析 |
| `src/kernel/detail/acl/windows/tests.rs` | ACL 纯逻辑单测：ACE 布局解析、身份判等、继承位、通用位展开与覆盖、完整性标签解析与挡写判据 |