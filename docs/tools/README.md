# tools（工具与围栏）

> 系统工具、角色表与一次工具执行的围栏。
> 系统侧的门户是 [SYSTOOL.md](../../SYSTOOL.md)（入口与真相源表）；本目录是它的细则。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：系统工具总表与角色表（`systools/`）、内置文件工具（read/write/edit/patch/search）的执行编排与纯规则、工具参数契约、补丁通道、外部工具进程的执行、**围栏策略与平台实现**、授权记录与撤销（写 ACL 前先落台账；写后核对**覆盖标准、对象与回调 ACE**，身份 = ACE 类型 + SID + 权限位 + 对象 GUID，布局认不出的如实计数；失败回滚；收尾按台账还原或精确撤销，并在产品根内回收孤儿授权）；**按条处置**与台账清单（列出快照路径 + 时间、根外授权 SID/路径/权限位、profile 与“当前实际 ACE 与台账对不对得上”的差异；按路径只还原一条、按 SID + 路径只撤一条——**含台账外的根外残留**、按名只删一个 profile）；**归属与启动对账**（每条授权 / profile 记下归属：pid + 进程创建时刻，防 PID 复用；台账写事务用**进程内互斥 + 跨进程文件锁**串行；启动期按归属回收“明确已死”的陈旧授权、无法判定的报告后跳过——不询问用户、不阻塞启动，失败保留供重试）。

**不管**：不放领域语义（角色是系统的身份，不是业务概念）；不选模型；不碰会话流水。

## 二、入站契约与状态归属

`api::Tools`（按角色发放工具面 / 总表 / 自检）+ `api::ToolExec`（跑模块与内置工具、释放围栏）。出站端口 `SysIo` / `ToolRunner` / `FenceHost` / `SystoolsSource` **只由 `service/mod.rs` 持有**（R12）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`prompt`、`workspace`
- 谁在用我：`collab`、`conductor`、`session`、`slate`

## 四、改动本单元时必须同步

- 平台专属代码本地不编译——`FenceSpec` 字面量必须写全字段（跨平台字面量门禁）；改围栏 → `tests/<平台>/` 探针；改工具表 → `systools/` 与本目录 `tools-and-roles.md`。
- 围栏的写后核对、回滚与台账在 `src/capabilities/tools/detail/confine/windows/`：改落点形状（`GrantTarget`）或台账字段时，`prepare_fence` / `release_fence` / `clean` / `sweep_orphan_aces` 与按条处置（`catalog` / `restore_one` / `revoke_grant` / `remove_profile_one`）要一起改。
- 改**归属字段**或启动对账时，`record.rs` 的 `current_owner` / `owners_state` / `reconcile` / `release_fence` / `clean` 与入口 `--fence-reconcile` 要一起改；启动对账的接线在 `src/main.rs`。
- ACE 的读法只有一处：`acl.rs` 的 `ace_parts` 按 ACE 头算 SID 偏移（标准 ACE 与回调 ACE 从第 8 字节起，对象 ACE 再加 Flags(4) 与在场的 GUID）。改覆盖的 ACE 类型或身份形状时，写后核对、诊断转储与 `windows/tests.rs` 的对象 ACE 往返探针要一起看。
- 容器里**列目录**的判据：同一个授权叶子上，cmd 的 `dir` 与 PowerShell 的 `Get-ChildItem` 会被拒，
  而 cmd 的 `for` 枚举与 python 的 `os.listdir` / `os.scandir` 正常（真机机制矩阵实测：叶子 DACL
  给容器的是 Modify，写读往返也通）。所以"列自己的产物"用 `for` 枚举钉住
  （`container_roundtrip_sees_leaf_but_not_parent_content` 的第四步），`dir` 不是可依赖的列举手段。
- 命令里的解释器有**基线**：`node <文件>` 的命令由围栏注入 `NODE_OPTIONS=--preserve-symlinks --preserve-symlinks-main`
  ——node 的 `fs.realpathSync` 先 lstat 盘卷根、再逐级 lstat 祖先前缀，而这两类落点按设计都不在可达范围
  （卷根属主是系统、非管理员改不动；祖先链只靠令牌的「按名穿过」特权，管不到显式 lstat），少了它进程在脚本执行前就 EPERM 死。
  只在本平台注入（Landlock 不管 stat、seatbelt 已给祖先放行 `file-read-metadata`）；判据与 `interpreter_dirs` 共用
  同一份 PATH 解析（`command_programs_in`），而白名单随命令而变——`--print-fence-env` 与探针都按同一条命令问。
- 授权面按**注入的事实**派生：`<module>/userdata/` 由 workspace 在**载入时确保存在**（产品唯一的自动写盘，幂等；建不了如实标注、不阻断加载）并随沙箱注入（domain 不读盘）；事实为否（建不了）时不进 `rw`，`standalone` 的缺省工作目录退回模块根。`prepare_fence` 也会跳过不存在的落点、不判整次失败。
- 核心代理系统工具的待实现规划清单：`systool_gaps.yaml`（仓库根；不是当前工具表，也不替代测试缺口账）。

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
- 数据边界父目录的只读属性（容器里判「这个目录在不在」用它；缺了会让模块自己的「父目录不存在就先建」走偏，
  那是**命令自己的报错**，不是围栏失守）；
- 不存在的落点（没有 `userdata/` 的模块、派生与执行之间的竞态）：跳过，不进结论。

**行为**（`prepare_fence` → `FencePrep` → `ProcTools`）：

- **必要落点授不上：不许降级**。经统一裁决通道（见 [session-model.md](../session/session-model.md) 的
  「请用户裁决」）推一条消息（写清哪一环、哪个目录、缺什么前提、怎么补）与两条选项：
  `fence_unfenced_once`（本轮无围栏跑一次——回执与 stderr 都**如实标为无围栏**）/
  `fence_abort`（放弃这次调用），并在这一问上声明"没人答就当我选了放弃"（`on_unanswered`）。用户拒绝、
  没人答、用户按停止（整队作废 = 拒绝）一律**不执行**（fail-closed）；回执把"用户拒绝"与"没人答"分开写清楚
  （后者说清是这一趟没有可回答的前端，还是按声明的默认项办的、**不是用户答的**）。
- **构不出可用选项**（除「放弃」外没有一条真能执行的）时**不发起裁决**，改为**停掉这个会话 + 落一条警告**
  （契约禁止置灰）；停过一次之后不再推卡。
- **可选落点授不上只记事实**：留一条 stderr 脚印，不牵动这次执行。
- **未授权档位不是这条路**：用户没开 `fence_write` 时外层根本不写权限项，那是用户自己选的档位，
  如实降级并在启动报告里说明能力等级。
- **回执文案来自提示词册**（`tool_texts.tool_fence_blocked` / `tool_fence_unfenced`）：它们随 `[工具结果]` 进模型上下文。
- **围栏里失败的统一说明**：工具在围栏里非零退出时，守门进程补一条**不分语言**的话（可达范围 + 范围外被围栏拒绝）；
  成功不打扰。语言自己的报错仍原样透传（各语言的 permission denied 就是这一层），产品不识别具体错误、也不生成配置。

**平台**：只有 Windows 有「外层授权写 ACL」这一步；Linux 的 Landlock 与 macOS 的 seatbelt 在守门进程里自足，
那里没有 `prepare_fence`（不是存在但空转）。

**启动对账只回收归属明确已死的条目**：`reconcile` 走台账、**不调用** `sweep_orphan_aces` / `sweep_profiles`
（那两条无归属、会误伤在跑实例）；孤儿清扫只在用户手动的 `--fence-clean`。

**真机验收**：`unwritable_interpreter_dir_never_silently_runs_unfenced`（`ProcTools` 的用例，`--fence-live` 才跑，
要可执行文件已构建）断言三条真机行为：选「跑一次」才跑且回执如实标为无围栏 / 选「放弃」不执行 /
没有可回答的前端也不执行。**已在真机通过**：解释器装在属主不是当前用户的目录里（系统级安装，当前用户对那个目录
没有写 DACL 的权限）时，授权写不进（错误码 5），三条行为如上。解释器目录授得进（构造不出这一态）就如实 `env-skip`。
## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`tools-and-roles.md`](tools-and-roles.md) | 系统工具、角色（身份）与「谁能用哪些工具」 |