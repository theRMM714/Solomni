# kernel（机制型业务）

> 没有领域语义的机制；依赖图最底层。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：

- 运行日志端口、宿主探测端口、**提问端口**（请用户裁决）。
- 生成中作业的取消表。
- 跨业务共享的**事实类型**与路径的对外书写形式。
- **外部进程执行**：一次性的 `ProcessRunner` 与长驻的 `SessionHost`。
- **长驻子进程的账与回收**（`procs`）。
- **围栏机制**（`confine`）：可达范围、出站网络、进程树、平台实现与 Windows 授权台账。

**不管**：

- **不认识任何能力 / 呈现层 / 入口层**。
- 不放有领域语义的类型——它不需要知道什么是回合、回复、工具执行。
- **不决定**可达范围（策略由调用方派生后传入）。

## 二、入站契约与状态归属

`api`（`SessionId` / `Tier` / `ExecSpec` / `ToolOutcome` / `Ask` / `AskOutcome` / `FenceSpec` / `DEFAULT_LLM_TIMEOUT_SECS` / `valid_capability` / `slash` / `JobRegistry`）、
`ports`（`Log`、`HostProbe`、`ToolHandler`、`AskUser`、`PathCodec`、`ProcessRunner`、`SessionHost`、`ProcessTexts`、`FenceHost`——R12 的例外：全项目共享）、
`domain`（事实类型、路径书写、取消表、围栏描述符）、
`detail`（`file_log`、`host_probe`、`outbound`、`unpack`、`process`、`procs`、`session`、`confine`）。

`AskUser` 是「需要用户裁决的机制请用户裁决」的**唯一**入口，工具执行层与围栏用它。

- 形状：`ask(问题) -> 选项 id`，**阻塞**。
- `halt(为什么)` = 停会话 + 落警告；调用方只在有可回答前端时用它，没有前端按 fail-closed 收场。
- 实现在会话侧（`conductor` 的 `SessionAsk`），走会话的**统一裁决通道**——见 [docs/session/session-model.md](../session/session-model.md) 的「请用户裁决」。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——依赖图最底层或纯领域）
- 谁在用我：`collab`、`conductor`、`llm`、`modules`、`prompt`、`registry`、`residents`、`session`、`tools`、`vm`、`workspace`

## 四、改动本单元时必须同步

- 共享事实类型（R6）一改，所有使用者同改；路径拼接规则见 `ARCHITECTURE.md` §八（禁止把分隔符写进字符串）。
- 平台专属代码本地不编译——`FenceSpec` 字面量必须写全字段（跨平台字面量门禁）；改围栏 → `tests/<平台>/` 探针。
- 围栏的写后核对、回滚与台账在 `src/kernel/detail/confine/windows/`：改落点形状（`GrantTarget`）或台账字段时，`prepare_fence` / `release_fence` / `clean` / `sweep_orphan_aces` 与按条处置（`catalog` / `restore_one` / `revoke_grant` / `remove_profile_one`）要一起改。
- 改**归属字段**（含会话租约 `Sandbox.session` → `FenceSpec.lease` → `Owner.lease`）或启动对账时，`record.rs` 的 `current_owner` / `owners_state` / `reconcile` / `release_fence` / `clean` 与入口 `--fence-reconcile` 要一起改；启动对账的接线在 `src/main.rs`。
- ACE 的读法只有一处：`acl.rs` 的 `ace_parts` 按 ACE 头算 SID 偏移（标准 ACE 与回调 ACE 从第 8 字节起，对象 ACE 再加 Flags(4) 与在场的 GUID）。改覆盖的 ACE 类型或身份形状时，写后核对、诊断转储与 `windows/tests.rs` 的对象 ACE 往返探针要一起看。
- 容器里**列目录**的判据：同一个授权叶子上，cmd 的 `dir` 与 PowerShell 的 `Get-ChildItem` 会被拒；cmd 的 `for` 枚举与 python 的 `os.listdir` / `os.scandir` 正常。所以"列自己的产物"用 `for` 枚举钉住（`container_roundtrip_sees_leaf_but_not_parent_content` 的第四步），`dir` 不是可依赖的列举手段。
- 命令里的解释器有**基线**：`node <文件>` 的命令由围栏注入 `NODE_OPTIONS=--preserve-symlinks --preserve-symlinks-main`——node 的 `fs.realpathSync` 会 lstat 卷根与祖先前缀，而这两类落点按设计不在可达范围；少了它，进程在脚本执行前就 EPERM 死。只在 Windows 注入（Landlock 不管读属性、seatbelt 已给祖先放行 `file-read-metadata`）；判据与 `interpreter_dirs` 共用同一份 PATH 解析（`command_programs_in`）。
- **真机测试（`--fence-live`）**：`unwritable_interpreter_dir_never_silently_runs_unfenced`（`ProcTools` 的用例）断言三条：选「跑一次」才跑且回执如实标为无围栏；选「放弃」不执行；没有可回答的前端也不执行。
- 改 `procs` 的登记字段或回收判据时，本页的「长驻子进程的账与回收」、`src/main.rs` 的启动对账、`--doctor` 的 `processes` 字段与 `GET /api/processes` 的响应要一起看。

## 五、围栏落点：必要与可选

**授权**（写目录 ACL）只有 Windows 的容器围栏需要：外层进程一次性把「可达范围」逐条授给容器身份，落点清单由
`grant_targets` 给出（叶子 + 直接父目录的只读属性）。**必要 / 可选的判据只有一处**：`domain/fence.rs` 的
`FencePart::necessary`（枚举里 `AuthorizedRead` 与 `Parent` 两条就是可选的）。

**必要**（缺了这次命令在容器里起不来，或这次执行根本做不了该做的事）：

| 落点 | 缺了会怎样 |
| --- | --- |
| 解释器安装目录 | 容器里连解释器都起不来 |
| 模块目录 / 工具进程的工作目录 | 工具脚本与它的依赖在这里，命令的起点也在这里 |
| 私有沙箱 | `HOME` / `TEMP` 的落点：进程连临时文件都落不下 |
| 其余数据边界（共享主副本、模块 `userdata/`） | 这次执行要读写的根：读不到输入、写不进产物 |
| 容器身份 / 授权台账 | 围栏本身的前提（派生不出 SID、台账落不下就先不动本机权限项） |

**可选**（缺了命令照跑，只是可达范围小一点）：

- 用户显式授权的主机只读根（`.home/settings.yaml` 的 `sandbox.policy.host_read_roots`）；
- 数据边界父目录的只读属性（容器里判「这个目录在不在」用它；缺了会让模块自己的「父目录不存在就先建」走偏，那是**命令自己的报错**，不是围栏失守）；
- 不存在的落点（没有 `userdata/` 的模块、派生与执行之间的竞态）：跳过，不进结论。

**行为**（`prepare_fence` → `FencePrep` → `ProcTools`）：

- **必要落点授不上：不许降级**。经统一裁决通道（见 [session-model.md](../session/session-model.md) 的「请用户裁决」）推一条消息：哪一环、哪个目录、缺什么前提、怎么补。
  - 两条选项：`fence_unfenced_once`（本轮无围栏跑一次——回执与 stderr 都**如实标为无围栏**）/ `fence_abort`（放弃这次调用）。
  - 这一问声明 "没人答就当我选了放弃"（`on_unanswered` = `fence_abort`）。
  - 用户拒绝、没人答、用户按停止：一律**不执行**（fail-closed）。
  - 回执把"用户拒绝"与"没人答"分开写清楚。
- **构不出可用选项**时**不发起裁决**（契约禁止置灰）：有可回答的前端就**停掉这个会话 + 落一条警告**，没有前端只如实拒绝这次调用；停过一次之后不再推卡。
- **可选落点授不上只记事实**：留一条 stderr 脚印，不牵动这次执行。
- **未授权档位不是这条路**（仅 Windows）：用户没开 `sandbox.policy.windows.write_acl` 时外层根本不写权限项，容器围栏不装；若策略要求强制则拒绝，否则按无围栏执行并在启动报告标出。Linux / macOS 没有外层授权这一步，围栏照常装。
- **启动门（组合根）**：默认 CLI 与 `-webUI` 都先判 `require` vs 本次实际，并把「未授权」与「本机能力缺失」分开报；未满足时只能本次降级 / 重设 / 退出。没有可回答的前端、或 `on_unavailable=refuse` 且不重设 = 退出，不会启动前端。首次启动（没有 `.home/settings.yaml`）在 Windows 先问一次是否授权写 ACL；选否不写盘、进入启动门。
- **回执文案由进程机制自己的代码返回**：`kernel/detail/process.rs` 的 `CodeProcessTexts`（围栏拒绝/放行、超时、stderr、截断）；`ProcessTexts` 只是窄端口，生产实现与机制同文件。
- **围栏里失败的统一说明**：工具在围栏里非零退出时，守门进程补一条**不分语言**的话（可达范围 + 范围外被围栏拒绝）；成功不打扰。

**平台**：只有 Windows 有「外层授权写 ACL」这一步；Linux 的 Landlock 与 macOS 的 seatbelt 在守门进程里自足，那里没有 `prepare_fence`（不是存在但空转）。三个平台各自的强制项与降级条件见下「工具进程与守门协议」。

**启动对账只回收归属明确已死的条目**：`reconcile` 走台账、**不调用** `sweep_orphan_aces` / `sweep_profiles`（那两条无归属、会误伤在跑实例）；孤儿清扫只在用户手动的 `--fence-clean`。

**释放按会话租约**：`release_fence` 只撤"当前进程 + 当前会话"的那一份归属；同名 agent 的另一个并发会话仍持归属时，ACE 与快照都留着。解释器基线与容器 profile 不属于任何会话（进程级归属），不受单会话释放影响。

## 六、工具进程与守门协议

一次工具执行（`ProcessRunner` 的实现 `ProcTools`）：

- 外层用**当前程序自己**当守门进程：`--fence-run <FenceJob JSON> -- <命令>`。
- `FenceJob` = `FenceSpec` + `prepared`（外层是否已写完本机授权）+ `home`（容器 profile 的台账落点）+ `policy`（本进程的强制项与降级许可）；扁平成一个 JSON 传给守门进程。
- 命令作为**一个 argv** 交给守门进程，外层不拼 shell；参数 JSON 经 stdin 送进工具进程，不进命令行。
- 守门进程用系统 shell 执行模块作者写的命令行（`sh -c` / `cmd /C`）——命令行是模块声明的一部分。
- 环境白名单：外层 `env_clear`，只注入 `fence_env`（PATH / PATHEXT / 系统根 / LANG / TZ 等，`HOME` 与 `TEMP` 指向该 agent 私有沙箱）与模块隐秘字段；其它环境与凭据不进工具进程。
- 收尾：stdout / stderr 分别读；超时 30 秒（`ProcTools::timeout`）后连根杀树；输出上限 16000 字符。超时、截断、围栏失败的文案经 `ProcessTexts` 注入，机制不硬编码。
- 围栏装不上（本机不支持 / 容器起不来）：命中 `require_fs` / `require_net` 且未同意降级时，守门进程以 `FENCE_FAILED` **拒绝执行**；已同意降级才如实降级继续。Linux 的文件系统与网络两项**独立**判定，一项装不上不影响另一项。能力等级已在启动报告里说明。
- Windows 外层未授权（`sandbox.policy.windows.write_acl=false`）：容器不装；启动门要求强制时只能本次降级 / 重设 / 退出，否则按无围栏执行；全局网络开关不生效于容器，启动报告的「本次实际」已标出。
- Windows 未授权时段先自检（`confine::verify`）：`EnvUnavailable` = 本机不支持，降级照跑；`Broken` = 自检说机制有效却装不上（我们写错了），**拒绝执行**，不按无围栏跑。
- 固定退出码 `111`（`FENCE_FAILED`）用于：`FenceJob` JSON 非法、守门进程里工具命令起不来、工具被信号结束拿不到退出码，或全局策略要求强制而机制装不上（拒绝执行）。
- 围栏生效且工具非零退出：守门进程往 stderr 补一条边界说明（可达范围 + 实际网络状态：开 / 关 / 未生效 + 范围外被围栏拒绝）；成功不打扰。

平台差异：

| 平台 | 文件系统 | 出站网络 | 机制 |
| --- | --- | --- | --- |
| Linux | Landlock（内核不支持或规则被拒时如实降级） | 按 `net`（真 = 不装网络过滤器；假 = seccomp 拒绝非 AF_UNIX / AF_NETLINK socket，TCP/UDP/raw 与回环都出不去） | 守门进程内装 Landlock 与 seccomp |
| macOS | seatbelt（`sandbox_init` 是私有且已弃用的 ABI；失效时如实降级） | 按 `net`（真 = 放行 `network-outbound`；假 = 拒绝 `network*`） | 守门进程内装 seatbelt；祖先元数据放行 `file-read-metadata` |
| Windows | AppContainer（外层 `prepare_fence` 把可达范围写进目录 ACL） | 按 `net`（真 = 授 `internetClient` 公网出站；假 = 不授） | 守门进程建容器 profile 并在容器内跑；作业对象管进程树与进程数上限 |
| 其它 | 无（如实降级） | 无 | 只有超时与整棵树终止 |

- **网络边界**：全局 `sandbox.policy.network` 是唯一开关。平台落点：
  Windows 授 `internetClient`（只放公网出站；不授 private network / loopback，也不做 loopback 豁免）；
  macOS `net=true` 放行 `network-outbound`（入站与绑定仍由 `deny default` 拒绝，回环出站可用），`net=false` 拒绝 `network*`；
  Linux `net=true` 不装网络过滤器（不限制入站/绑定，这是与 Windows / macOS 的平台差异），`net=false` 用 seccomp 拒绝非 AF_UNIX / AF_NETLINK socket。
  Windows 未授权 `windows.write_acl` 时容器不装，网络不生效（启动报告的「本次实际」会标出）。
- 进程树：Unix 起进程时开独立进程组，超时 / 退出时对负 pid 发 `SIGKILL`；Windows 把工具进程放进守门进程的作业对象（`KILL_ON_JOB_CLOSE`），守门进程一死整棵树随之消亡。
- **本机能力**与**本次实际**分开报（见 `src/main.rs` 的启动报告）：`sandbox.policy.windows.write_acl` 只决定 Windows 是否写目录 ACL；Linux / macOS 的围栏在守门进程里自足。所以「本次实际」在 Linux / macOS 上等于本机能力；Windows 未授权时 fs/net 为假，进程树、资源上限与环境白名单照旧生效。
- **进程级降级**：`FenceJob.policy` 携带 `require_fs` / `require_net` / `allow_degraded`；机制装不上时，`allow_degraded=false` 一律以 `FENCE_FAILED` 拒绝（所有平台一致），已同意降级才继续。降级只活在当前进程，不落盘。

## 七、长驻子进程的账与回收（`procs`）

账本（`.home/processes.json`，`ProcessBook`）：

- 一条登记：`pid` / `since`（unix 秒，只作呈现与先后判断）/ `purpose` / `owner` / `marker`（命令行里应出现的标记）/ `created`（身份）/ `guard_note`（父死子亡没挂上的原因，空 = 挂上了）。
- 身份按平台用**能证明的判据**：Unix 是命令行里的 `marker`，Windows 是进程创建时间 `created`（`GetProcessTimes`）。
- `purpose` 的当前取值：`guest`（QEMU）、`share:smb`、`share:virtiofs`、`resident`（常驻服务的守门进程）。
- `owner`：QEMU 记租约；共享 helper 记落点（首个挂载 / guest）；常驻服务记 `resident/<模块>/<服务>`。
- 打开时读不懂就当空账；父目录不存在就补建。落盘用同目录临时文件 + rename。
- 同一 pid 只留最新一条；`list` 按 `(since, pid)` 升序。

父死子亡（`procs::guard` / `bind` / `watch`，接线走 `attach`）：

- Unix 起进程前开独立进程组（`process_group(0)`），收树不牵连核心。
- Linux：`pre_exec` 里设 `PR_SET_PDEATHSIG=SIGKILL`，核心一死由内核收掉子进程。
- macOS：`procs::watch` 另起本程序的守门进程 `--proc-watchdog <核心 pid> <子进程 pid>`；它用 kqueue 的 `EVFILT_PROC` / `NOTE_EXIT` 同时盯核心与子进程，核心先退就按进程组 `SIGKILL` 子进程。守门进程清空环境、独立进程组、不挂 PDEATHSIG；起不来如实返回原因，不阻断启动。
- Windows：`procs::bind` 把子进程绑进一个 `KILL_ON_JOB_CLOSE` 作业对象；核心一死，作业关闭，子进程跟着死。
- `procs::attach` 是起进程之后唯一的接线点（`bind` + `watch`，并读一次身份）：挂不上只打一条 `[进程]` 并把原因交回调用方，**不阻断启动**。调用方把它记进账的 `guard_note`——`--doctor` 与界面据此把该条标成「核心被强杀会留下孤儿」，下次启动对账再按身份收掉它。
- Unix 上 `procs::spawn` 统一切到常驻的 `solomni-proc-launcher` 线程上 spawn：`PR_SET_PDEATHSIG` 绑的是**发起 spawn 的线程**；在 Web 每请求一线程或短命工作线程上直接 spawn，线程一退就会把 QEMU / 共享 helper / 常驻服务守门进程一并杀掉。

启动对账（`ProcessBook::reconcile`，组合根在会话开工前跑一次）：

- 进程已不在（`alive` 为假）：删掉条目。
- Windows：读该 pid 的创建时间，与账上 `created` 对得上才 `taskkill /PID <pid> /T /F` 收掉；对不上按 pid 已被复用作废；账上没有 `created`（更早的账）只报不动手。
- Unix：读该 pid 的命令行；含该条 `marker` 才 `terminate`（按进程组先 TERM，最多等 3 秒，再 KILL——守门进程拉起的服务跟着收）；不含就按 pid 已被复用作废条目。
- 报告三项：`dropped` / `killed` / `alive`；有动静才打印一行。

存活探测与终止：

- 存活探测：Unix 用 `kill -0`；Windows 用 `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `GetExitCodeProcess`。
- 终止：Unix 按进程组先 TERM 再 KILL（不是组长的就发单 pid）；Windows 用 `taskkill /PID <pid> /T /F`。

接线点：

- QEMU 引擎、SMB helper、virtiofsd 在起进程处走 `guard` → `spawn` → `attach`，并在 `ProcessBook` 里 `record`（带 `created` / `guard_note`）；销毁时 `forget`。
- 常驻服务的守门进程（`src/kernel/detail/session.rs` 的 `ProcessSessions::open`）同样走 `guard` → `spawn` → `attach`，并登记进同一份账本（`purpose = resident`）：会话收掉时注销，核心被强杀留下的孤儿由启动对账按身份收掉。
- 观测：`--doctor` 的 `processes` 与 `GET /api/processes` 读同一份账本，每条带 `orphanRisk` / `guardNote`；覆盖 QEMU、共享 helper 与常驻服务守门进程。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
