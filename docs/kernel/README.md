# kernel（机制型业务）

> 没有领域语义的机制；依赖图最底层。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：运行日志端口、宿主探测端口、**提问端口**（请用户裁决）、生成中作业的取消表、跨业务共享的**事实类型**、路径的对外书写形式；**外部进程执行（一次性的 `ProcessRunner` 与长驻的 `SessionHost`）与围栏机制**（`confine`：可达范围、断网、进程树、平台实现与 Windows 授权台账）。

**不管**：**不认识任何能力 / 呈现层 / 入口层**；不放有领域语义的类型——它不需要知道什么是回合、回复、工具执行；**不决定**可达范围（策略由调用方派生后传入）。

## 二、入站契约与状态归属

`api`（`SessionId` / `Tier` / `ToolOutcome` / `Ask` / `FenceSpec` / `DEFAULT_LLM_TIMEOUT_SECS` / `slash` / `JobRegistry`）、
`ports`（`Log`、`HostProbe`、`ToolHandler`、`AskUser`、`ProcessRunner`、`SessionHost`、`ProcessTexts`、`FenceHost`——R12 的例外：全项目共享）、`domain`（事实类型、路径书写、取消表、围栏描述符）、
`detail`（`FileLog`、`HostProbe`、`process`、`confine`）。

`AskUser` 是「需要用户裁决的机制请用户裁决」的**唯一**入口（形状：`ask(问题) -> 选项 id`，**阻塞**；
`halt(为什么)` = 构不出可用选项时停会话 + 落警告）：工具执行层与围栏用它，实现在会话侧（`conductor` 的 `SessionAsk`），
走的是会话的**统一裁决通道**——见 [docs/session/session-model.md](../session/session-model.md) 的「请用户裁决」。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——依赖图最底层或纯领域）
- 谁在用我：`conductor`、`llm`、`prompt`、`registry`、`session`、`tools`、`workspace`

## 四、改动本单元时必须同步

- 共享事实类型（R6）一改，所有使用者同改；路径拼接规则见 `ARCHITECTURE.md` §八（禁止把分隔符写进字符串）。
- 平台专属代码本地不编译——`FenceSpec` 字面量必须写全字段（跨平台字面量门禁）；改围栏 → `tests/<平台>/` 探针。
- 围栏的写后核对、回滚与台账在 `src/kernel/detail/confine/windows/`：改落点形状（`GrantTarget`）或台账字段时，`prepare_fence` / `release_fence` / `clean` / `sweep_orphan_aces` 与按条处置（`catalog` / `restore_one` / `revoke_grant` / `remove_profile_one`）要一起改。
- 改**归属字段**（含会话租约 `Sandbox.session` → `FenceSpec.lease` → `Owner.lease`）或启动对账时，`record.rs` 的 `current_owner` / `owners_state` / `reconcile` / `release_fence` / `clean` 与入口 `--fence-reconcile` 要一起改；启动对账的接线在 `src/main.rs`。
- ACE 的读法只有一处：`acl.rs` 的 `ace_parts` 按 ACE 头算 SID 偏移（标准 ACE 与回调 ACE 从第 8 字节起，对象 ACE 再加 Flags(4) 与在场的 GUID）。改覆盖的 ACE 类型或身份形状时，写后核对、诊断转储与 `windows/tests.rs` 的对象 ACE 往返探针要一起看。
- 容器里**列目录**的判据：同一个授权叶子上，cmd 的 `dir` 与 PowerShell 的 `Get-ChildItem` 会被拒，而 cmd 的 `for` 枚举与 python 的 `os.listdir` / `os.scandir` 正常（真机机制矩阵实测）。所以"列自己的产物"用 `for` 枚举钉住（`container_roundtrip_sees_leaf_but_not_parent_content` 的第四步），`dir` 不是可依赖的列举手段。
- 命令里的解释器有**基线**：`node <文件>` 的命令由围栏注入 `NODE_OPTIONS=--preserve-symlinks --preserve-symlinks-main`——node 的 `fs.realpathSync` 会 lstat 卷根与祖先前缀，而这两类落点按设计不在可达范围，少了它进程在脚本执行前就 EPERM 死。只在本平台注入（Landlock 不管 stat、seatbelt 已给祖先放行 `file-read-metadata`）；判据与 `interpreter_dirs` 共用同一份 PATH 解析（`command_programs_in`）。
- **真机验收**：`unwritable_interpreter_dir_never_silently_runs_unfenced`（`ProcTools` 的用例，`--fence-live` 才跑）断言三条真机行为：选「跑一次」才跑且回执如实标为无围栏 / 选「放弃」不执行 / 没有可回答的前端也不执行。

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

- 用户显式授权的只读根（`.home/settings.yaml` 的 `fence_read`）；
- 数据边界父目录的只读属性（容器里判「这个目录在不在」用它；缺了会让模块自己的「父目录不存在就先建」走偏，那是**命令自己的报错**，不是围栏失守）；
- 不存在的落点（没有 `userdata/` 的模块、派生与执行之间的竞态）：跳过，不进结论。

**行为**（`prepare_fence` → `FencePrep` → `ProcTools`）：

- **必要落点授不上：不许降级**。经统一裁决通道（见 [session-model.md](../session/session-model.md) 的「请用户裁决」）推一条消息（写清哪一环、哪个目录、缺什么前提、怎么补）与两条选项：`fence_unfenced_once`（本轮无围栏跑一次——回执与 stderr 都**如实标为无围栏**）/ `fence_abort`（放弃这次调用），并在这一问上声明"没人答就当我选了放弃"（`on_unanswered`）。用户拒绝、没人答、用户按停止一律**不执行**（fail-closed）；回执把"用户拒绝"与"没人答"分开写清楚。
- **构不出可用选项**时**不发起裁决**，改为**停掉这个会话 + 落一条警告**（契约禁止置灰）；停过一次之后不再推卡。
- **可选落点授不上只记事实**：留一条 stderr 脚印，不牵动这次执行。
- **未授权档位不是这条路**：用户没开 `fence_write` 时外层根本不写权限项，如实降级并在启动报告里说明能力等级。
- **回执文案来自提示词册**（`tool_texts.tool_fence_blocked` / `tool_fence_unfenced`）：经 `ProcessTexts` 端口注入（进程机制不硬编码文案）。
- **围栏里失败的统一说明**：工具在围栏里非零退出时，守门进程补一条**不分语言**的话（可达范围 + 范围外被围栏拒绝）；成功不打扰。

**平台**：只有 Windows 有「外层授权写 ACL」这一步；Linux 的 Landlock 与 macOS 的 seatbelt 在守门进程里自足，那里没有 `prepare_fence`（不是存在但空转）。

**启动对账只回收归属明确已死的条目**：`reconcile` 走台账、**不调用** `sweep_orphan_aces` / `sweep_profiles`（那两条无归属、会误伤在跑实例）；孤儿清扫只在用户手动的 `--fence-clean`。

**释放按会话租约**：`release_fence` 只撤"当前进程 + 当前会话"的那一份归属；同名 agent 的另一个并发会话仍持归属时，ACE 与快照都留着。解释器基线与容器 profile 不属于任何会话（进程级归属），不受单会话释放影响。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
