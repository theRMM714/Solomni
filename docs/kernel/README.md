# kernel（地基库）

> 定位：没有领域语义的机制库；不是层，是依赖图最底层的地基。
> 效力：规范｜核对：2026-11-28｜依据：`src/kernel/api.rs`、`docs/kernel/mechanisms.yaml`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；机制与机制之间的层号在 `docs/kernel/mechanisms.yaml`（门禁 `tests/gates/structure/kernel-mechanisms.js`），分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：运行日志端口、宿主探测端口、**提问端口**（请用户裁决）、**时间**（`clock`：此刻的本机民用时间、到分钟流水戳，以及记时刻用的 `now_secs` / `now_ms`）。
- 管：生成中作业的取消表。
- 管：**内容指纹**（`hash`：SHA-256 增量实现，`content_hash` / `Sha256` / `is_content_hash`）——版本化、工具读写、运行包与素材指纹共用同一份。
- 管：**回执预算**（`budget`：`ResultBudget` / `Continuation` / `cap_result_with` / `MAX_RESULT_CHARS`）——模型可见的截断只有这一处口径，执行端口把它当参数往下传。
- 管：**路径桥**（`path_bridge`：`PathBridge` / `PathDialect`）——宿主路径与客机路径互译，虚拟机的路径书写与宿主的视图路径共用同一份。
- 管：**端口载荷形状**（`contracts`：`SessionId` / `ToolOutcome` / `Ask`）与路径的对外书写形式；其余跨层共享形状归环 1 单元（`Side` / `TargetSpec` / `JobState` 在 `exec_model`）。
- 管：**外部进程执行**——一次性的 `ProcessRunner` 与长驻的 `SessionHost`；围栏裁决经 `FenceGate` 原样转交，机制不解释裁决的业务变体。
- 管：**长驻子进程的账与回收**（`procs`）。
- 管：**围栏机制**（`confine`）——可达范围、出站网络、进程树、平台实现与 Windows 授权台账。
- 不管：**不认识任何单元 / 呈现层 / 根**。
- 不管：不解释共享形状的业务语义——它不需要知道什么是回合、回复、工具执行。
- 不管：**不决定**可达范围（策略由调用方派生后传入）。

## 入站契约与状态归属

- 入站面：`api`（按对外概念分文件：`fence` / `note` / `run` / `types` / `clock` / `fs` / `host_probe` / `i18n` / `locus` / `process` / `procs` / `lock` / `view_drive` / `actor` / `jobs` / `layout` / `name` / `net` / `path` / `platform` / `hash` / `budget` / `path_bridge`——见「六、对外面」）、`ports`（`Log`、`HostProbe`、`ToolHandler`、`AskUser`、`PathCodec`、`ProcessRunner`、`SessionHost`、`FenceHost`、`FenceGate`、`UserPath`——R12 的例外：全项目共享）、`contracts`（共享形状、围栏描述符、执行回执与回执说明）、`domain`（路径书写、取消表、沙箱模式、回执装配）、`detail`（`clock`、`file_log`、`fs`、`host_probe`、`io`（`env` / `net`）、`unpack`、`user_path`、`process`、`procs`、`session`、`i18n`、`confine`：门面 + `job` / `ledger` / `paths` / `prep`）。
- `AskUser` 是「需要用户裁决的机制请用户裁决」的**唯一**入口（`src/kernel/ports.rs`），工具执行层与围栏用它。
  - 形状：`ask(问题) -> 选项 id`，**阻塞**。
  - `halt(为什么)` = 停会话 + 落警告；调用方只在有可回答前端时用它，没有前端按 fail-closed 收场。
  - 实现在回合状态侧（`src/units/business/turn_state/domain/ask.rs` 的 `SessionAsk`），走会话的**统一裁决通道**——见 [docs/session/session-model.md](../session/session-model.md) 的「请用户裁决」。
- 状态归属：取消表、围栏台账与进程账都在本单元机制侧维护，业务状态不在此。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——依赖图最底层或纯领域）
- 谁在用我：`collab`、`runtime`、`llm`、`modules`、`prompt`、`registry`、`residents`、`session`、`tools`、`vm`、`workspace`

## 一、围栏落点：必要与可选

- 授权（写目录 ACL）只有 Windows 的容器围栏需要：外层进程一次性把「可达范围」逐条授给容器身份，落点清单由 `grant_targets` 从**计划**推出（每条授予的叶子 + 直接父目录的只读属性）。**必要 / 可选的判据只有一处**：`src/kernel/detail/confine/prep.rs` 的 `FencePart::necessary`（枚举里 `AuthorizedRead` 与 `Parent` 两条就是可选的）。`FencePart` 是**实现层词汇**（解释器基线、祖先链、容器身份这些只有 Windows 知道），不出现在对外契约里：调用方只拿 `FenceBlocked`。
- 必要（缺了这次命令在容器里起不来，或这次执行根本做不了该做的事）：

| 落点 | 缺了会怎样 |
| --- | --- |
| 解释器安装目录 | 容器里连解释器都起不来 |
| 模块目录 | 工具脚本与它的依赖在这里（只读子树）；命令的 cwd 落在私有沙箱或这个子树里，不再单列一条 cwd 授予 |
| 私有沙箱 | `HOME` / `TEMP` 的落点，也是**容器临时目录**（`Packages/<容器名>/AC/Temp`）的父目录：进程连临时文件都落不下 |
| 其余数据边界（共享主副本、模块私有区） | 这次执行要读写的根：读不到输入、写不进产物 |
| 容器身份 / 授权台账 | 围栏本身的前提（派生不出 SID、台账落不下就先不动本机权限项） |
| 落点的完整性标签 | 高于 Low 且带「不许向上写」时，DACL 授权再对容器也写不进：授权会报成功、命令却落不下产物 |

- 完整性标签是 DACL 之外的另一道检查：容器令牌恒为 Low，授写叶子带 Medium 及以上的 `SYSTEM_MANDATORY_LABEL_ACE`（`NO_WRITE_UP`）就被挡。`prepare_fence` 授权后逐叶子读一次标签（`src/kernel/detail/acl/windows/label.rs`，只读、免提权），命中按必要落点报 `FencePart::IntegrityLabel`。**只看叶子本身**：写一个对象只对它做完整性检查，祖先只被穿过；叶子继承到的标签会出现在它自己的 SACL 里。未打标签的对象不做这项检查——那是容器能写的正常基线。

- 可选（缺了命令照跑，只是可达范围小一点）：
  - 用户显式授权的主机只读根（`data/.home/settings.yaml` 的 `sandbox.policy.host_read_roots`）。
  - 数据边界父目录的只读属性（容器里判「这个目录在不在」用它；缺了会让模块自己的「父目录不存在就先建」走偏，那是**命令自己的报错**，不是围栏失守）。
  - 不存在的落点（私有区建不出来的模块、派生与执行之间的竞态）：跳过，不进结论。
- 行为（`prepare_fence` → `FencePrep` → `ProcTools`）：
  - **预检失败不写**：解释器根不可授权时 `prepare_fence` 直接给出必要落点缺失，不产生「写一半 / 回滚」的状态；回执只写真身路径。
  - **必要落点授不上：不许降级**。工具层的围栏裁决（`src/units/business/tools/service/fence.rs` 的 `ToolFenceGate`）经统一裁决通道（见 [session-model.md](../session/session-model.md) 的「请用户裁决」）推一条消息：哪一环、哪个目录、缺什么前提、怎么补。
    - 两条选项：`fence_unfenced_once`（本轮无围栏跑一次——回执与 stderr 都**如实标为无围栏**）/ `fence_abort`（放弃这次调用）。
    - 没有默认项：这一问不带「没人答就当我选了什么」，没人答就是没放行（`AskOutcome::NoAnswer`）。
    - 用户拒绝、没人答、用户按停止：一律**不执行**（fail-closed，`src/units/business/tools/service/fence.rs`）。
    - 回执把「用户拒绝」与「没人答」分开写清楚。
  - **构不出可用选项**时**不发起裁决**（契约禁止置灰，`src/units/business/tools/service/fence.rs`）：裁决端口回 `AskOutcome::NoOptions` 并落一条 `Notice`，**停不停由发起方定**——围栏有可回答的前端就自己停掉这个会话，没有前端只如实拒绝这次调用。
  - **可选落点授不上只记事实**：留一条 stderr 脚印，不牵动这次执行。
  - **视图盘让容器 cwd 不再需要祖先授权**（仅 Windows）：产品根映射成本实例独占的视图盘（一个空闲盘符，优先 Z:、被占则依次往下找；实测常见 S:/R:，别假设恒为 Z:），**视图写法目前只用于 shell（cwd == 私有沙箱）的容器 cwd 与程序名；模块工具沿用宿主 cwd，不参与视图盘收敛。**（模块工具的程序名里带分隔符的相对路径若译成视图写法，会被算成跨卷绝对路径，容器里反而调不起来。）shell 的容器 cwd 是 `<盘符>:\session\<工作>\<agent>`，模型拿到的路径也是；路径码把视图写法翻回宿主真实路径再判越界。映射根下还是宿主同一批文件，授权仍落在宿主路径上，但 PowerShell 逐级解析 cwd 只会经过产品根内的层级，产品根之外的祖先（盘根、用户目录等）不再需要任何 ACE。产品根上的一条 `(OI)(CI)` 读属性是唯一相关的机器级落点（见下一条）；视图盘由组合根在会话前分配（一次性命令 `runtime: none` 不分配）、正常退出释放；Ctrl+C / 关窗 / 注销 / 关机这类控制台终止由分配时装上的控制台钩子先撤映射与登记，崩溃 / 强杀留下的映射由下次启动清扫与 `--fence-clean` 回收（登记只认「pid + 进程创建时刻」，pid 被系统复用后创建时刻对不上，不算还有人用；`src/kernel/detail/view_drive/windows.rs`；门面 `src/kernel/detail/view_drive.rs` 只在 Windows 重导出实现，路径码出口 `kernel::api::view_codec` 在其余平台恒为 `None`）。
  - **授权主体分两个（仅 Windows）**：我们自己写的 ACE 一律授给**本产品的能力 SID**（`solomni.fence.baseline`，经 `DeriveCapabilitySidsFromName` 派生成 `S-1-15-3-*`，见 `src/kernel/detail/container/windows.rs` 的 `CAPABILITY_NAME` / `product_capability_sid`）——profile（`CreateAppContainerProfile`）与令牌（`SECURITY_CAPABILITIES`）两处都声明它，只有声明了的容器才拿得到这条只读。**不用 `ALL APPLICATION PACKAGES`（`S-1-15-2-1`）当授权主体**：那等于把读权限下放给本机所有 AppContainer 应用。系统目录（`System32` 等）自带的 AAPP 只读 ACE 不是我们写的、也改不动，所以解释器预检**两个主体都认**（`interpreter_baseline_present`），但只有能力 SID 那条算「我们已授」（`system_baseline_sid` 只用于识别，不用于写入）。真机判据：`tests/windows/probes/capability.rs` 的 `capability_sid_is_grantable_and_not_blanket`——带能力的容器读得到只授给该能力的文件、不带能力的读不到，且两者都写得了自己的工作目录。
  - **机器级一次性预授（仅 Windows）**：`--setup=fence`（或首次授权成功后自动一次）把两类落点一次写掉——产品根上的一条 `(OI)(CI)` 读属性（授给本产品能力 SID，覆盖根下全部既有与新建子项，视图盘根因此可解析；产品根之外的祖先不写）、`.tools/` 与 `data/runtimes/` 的递归只读。预授成功后顺带把 `sandbox.policy.windows.write_acl` 记进设置（授权轴与机器级缺项轴各看各的事实，预授失败就不记）；之后 `prepare_fence` 对这些落点全部命中已有 ACE、零写入；机器级条目不随会话回收，撤除走 `--fence-clean` 或按条撤销。入口 `src/kernel/detail/confine/windows/machine.rs`（`machine_plan` / `machine_missing` / `machine_ensure`），门面在 `src/kernel/detail/confine/prep.rs`。
  - **旧全应用包授权的退役（仅 Windows）**：`--setup=fence` 预授成功后顺带退役台账里**我们自己发过的**机器级 AAPP 条目（`retire_legacy_machine_grants`）：只认 `scope=machine` 且 SID 为 `S-1-15-2-1` 的条目，**先授后撤**——同一路径上能力 SID 的 ACE 覆盖得住这条的权限位（继承来的也算）才撤，覆盖不住就保留（解释器目录、产品根之外的祖先因此留着，要撤用 `--fence-revoke <SID> <路径>`）。系统目录自带的 AAPP ACE 不进台账，一条都不碰；盘卷根不动。浅路径先撤：父目录那条可继承 ACE 一撤，子目录继承到的副本由系统一并收走，子条目只销账、不再各写一次整树 ACL；撤与撤后核对都只看对象**自己带着**的 ACE（继承来的投影属于祖先那条授权，不在这一条里撤）。逐条开台账事务（锁只罩读—改—写），撤后核对残留；任一条失败就保留条目、如实报，重跑可再撤。首次授权后的自动那一趟只走 `machine_ensure`，不做这件慢清理。真机判据：`src/kernel/detail/confine/windows/tests/grant.rs` 的 `legacy_aapp_machine_grants_retire_only_after_capability_is_in_place`。
  - **三类根（仅 Windows）**：① **可信执行基座**——产品内工具链（`.tools/`）与运行包（`data/runtimes/<包>`）是只读二进制、不携带任何 scope 信息，一次性授给本产品能力 SID（与祖先链同路，不同于系统解释器目录自带的 AAPP），按**机器级**归属记台账（`scope=machine`、无归属，启动对账不回收、跨会话复用），**不进容器身份**；② **受保护数据 / 代码**——私有沙箱（`home`）、模块目录、模块私有区（`data/modules/<id>/userdata`）、用户授权只读根，按计划里的授予授给容器 SID；③ 系统基线（`C:\Windows` 等）。把工具链放进计划会让同一份 16k 文件按每个计划组合各授一次（本机实测 56s/次），属于类别错误。
  - **容器临时目录先建**（仅 Windows）：`prepare_fence` 在授权前建好 `<私有沙箱>/Packages/<容器名>/AC/Temp`——AppContainer 的 `GetTempPath2` **不读 TMP/TEMP**，编译器这类程序缺了它直接 abort（`Cannot create temporary file`）；目录在私有沙箱（`home`）之下，递归 ACE 覆盖它，不单列一条授权。`private` 为空时跳过并记一条事实。
  - **未授权档位不是这条路**（仅 Windows）：用户没开 `sandbox.policy.windows.write_acl` 时外层根本不写权限项，容器围栏不装；若策略要求强制则拒绝，否则按无围栏执行并在启动报告标出。Linux / macOS 没有外层授权这一步，围栏照常装。
  - **启动门（组合根）**：只有命令声明 `runtime: services` 的调用（默认 CLI、`--web`、会话、模块工具）先判 `require` vs 本次实际，`runtime: none` 的一次性命令不吃这道门；门把「未授权」与「本机能力缺失」分开报，非 TTY 时逐项列缺项与三条出路后以 1 退出；未满足时只能本次降级 / 重设 / 退出（`src/main.rs`）。没有可回答的前端、或 `on_unavailable=refuse` 且不重设 = 退出，不会启动前端。首次启动（没有 `data/.home/settings.yaml`）在 Windows 先问一次是否授权写 ACL；选否不写盘、进入启动门。授权后立刻跑机器级预授；后续启动只做 O(深度) 只读对账，缺项只提示 `--setup=fence`，不阻断。`require` + `write_acl=false` 的矛盾配置在加载 / 保存时即拒绝，不会进入门。
  - **回执 = 正文 + 结构化说明**：`RunOutcome` / `ToolOutcome` 的 `output` 只放进程原始输出；超时、退出码、信号、启动失败、围栏没装上都表达成 `Note`（稳定 ASCII 码 + 类型化参数 + `info` / `warning` / `error`），码表在 `src/kernel/contracts/note.rs`。kernel 只产出码与参数，**不构造业务文案**（R6）；文本在边缘按 `locales/<lang>.ftl` 渲染（`src/kernel/detail/i18n.rs`）。
  - **不声称失败原因**：越界拒绝发生在工具子进程内部（ACL / Landlock / seatbelt 都在那里），守门进程只看得到退出码与被本地化的 OS 文案。回执因此**只说跑没跑、退出码多少**，不写"被围栏拒绝"之类因果；可达范围这类事实由工具层结构化交给界面，不进模型回执。
- **意图与实现分家**：上层只交 `FencePlan`（`agent` / `lease` / `home` / `cwd` / `grants` / `net`）。每条授予是 `Grant { path, access, reach }`：
  - `access` 只有 `Read` / `Write` 两档——写蕴含读与执行（三平台都给不出「只收窄执行位」这种授予，设第三档就是假抽象）；
  - `reach` 只有 `Dir`（只这一层）/ `Tree`（整棵子树）——只有 Windows 的 DACL 能如实表达「不递归」，Landlock 的 path-beneath 只能按子树落实（如实放宽，不假装精确；Windows 侧的落实见 `src/kernel/detail/acl/windows/target.rs` 的 `grant_targets`，Linux 侧见 `src/kernel/detail/confine/linux.rs` 的 `install_rules`）；
  - `home` 隐含一条「写整棵子树」的授予（私有沙箱、`HOME` / `TEMP`、容器临时目录都在它下面），不再重复进 `grants`。
- 平台差异全部收在 `src/kernel/detail/confine.rs` 的 `trait Confine`（`capability` / `verify` / `effective_paths` / `run`）的四个实现里：Linux 落成 Landlock + seccomp，macOS 落成 seatbelt profile，Windows 落成 AppContainer + 目录 ACL，其它平台如实降级。**上层代码里没有围栏的 `cfg(windows)` 分支**。「先把授权写进本机」这一步只有 Windows 有（Unix 的围栏在守门进程里自足），所以它是 Windows 后端自己的入口 `prepare_fence`（`src/kernel/detail/confine/windows.rs`），不占 trait 的位置；三个调用点都在 Windows 专用文件里。三个平台各自的强制项与降级条件见「二、工具进程与守门协议」。
- **启动对账只回收归属明确已死的条目**：`reconcile` 走台账、**不调用** `sweep_orphan_aces` / `sweep_profiles`（那两条无归属、会误伤在跑实例）；孤儿清扫只在用户手动的 `--fence-clean`。台账里的机器级条目（`scope = machine`、没有归属）按设计也不回收：它们授给本产品能力 SID、跨会话复用，撤除走 `--fence-clean` 或按条撤销（`src/kernel/detail/ledger/windows/liveness.rs` 的 `entry_state`）。台账之外的本程序 profile 会以「只报告、不自动删」列出来（没有归属证明，删了会误伤在跑实例），处置入口同样是 `--fence-clean`。
- **台账写事务串行**：每条授权 / profile 带**归属**（pid + 进程创建时刻 + 会话租约），台账落在 `data/state/fence-grants.json`；写事务用**进程内互斥 + 跨进程文件锁**，两个进程不会同时改同一份台账。
- **锁只罩台账读—改—写**：`with_ledger` 在锁内 `load` → 记账 → `save`，返回即释放；慢的 ACL 写（`grant_verified` / `TreeSetNamedSecurityInfoW`）、`acl_scan`、`has_ace_for` 都在锁外。同进程的并发工具调用不因某一条整树重算互相排队，跨进程锁的持有时间与 ACL 写时长解耦（`src/kernel/detail/ledger/windows/store.rs`、`src/kernel/detail/ledger/windows/journal.rs`）。
- **围栏准备分步计时与进度**（仅 Windows）：`prepare_fence` 只对**真写** ACL 的落点记一步（标签固定词表：解释器目录 / 祖先链 / 受保护根 / 可信基座，不含用户路径），首次真写前 stderr 提示一次「首次为…补权限（可能要几十秒）」，完成后一条汇总（只报 ≥1 秒的类别；纯命中 / 总耗时 <1 秒保持安静）；耗时的围栏准备部分由工具层按总耗时补一条说明（码 `shell.place.prep`，≥1 秒才报；`src/kernel/detail/confine/prep.rs` 的 `FenceStep` / `FencePrep.steps`、`src/kernel/detail/process.rs` 的 `FenceTiming`）。
- **身份按会话与 agent、生命周期按会话**：Windows 的容器身份 = `会话租约 + agent + 计划指纹`（`home` / `cwd` / 每条授予的权限与路径；不含工具链、不含 net）——授予固定后同一 agent 的根集合恒定，同一身份的后续调用复用 SID 与 ACL。授权由会话持有：调用结束**不**释放，会话改配 / 删除时由 `release_by_lease` 按会话租约撤 ACE、删 profile；删除会话先停 guest、再删目录，然后把按租约撤权甩到后台（先删目录让根内授权走「路径不在 → 只销账」快分支）；进程退出导致后台线程被杀时，残留由 `reconcile` 按死归属回收。
  - **容器身份评估结论（仅 Windows）**：保留 AppContainer profile + 根集合指纹，不改全局 SID、也不每次调用换 SID（`src/kernel/detail/container/windows.rs` 的 `container_name` / `ensure_profile`）。理由：profile 必须在场才能拿到 AAPP 组（否则容器连系统解释器都打不开）；ACL 写成本按根集合计，同一根集合复用 profile 才能让后续调用零写；每次调用换 SID 会把授权成本推到每次调用；机器级只读交给产品能力 SID、不进根集合指纹，避免工具链 / 运行包按调用各授一次。边界：AppContainer 是同一用户内的 DACL 隔离，不是跨用户安全边界；profile 名是 FNV 指纹，不是秘密。真机判据：`tests/windows/probes/capability.rs` 的 `capability_sid_is_grantable_and_not_blanket`。一次性调用（构建 / 独立模块工具）没有会话，各自唯一租约，跑完走 `release_call`（含 `private/Packages/<名字>` 临时目录）。

## 二、工具进程与守门协议

- 一次工具执行（`ProcessRunner` 的实现 `ProcTools`）：
  - 外层用**当前程序自己**当守门进程：`--fence-run <FenceJob JSON> -- <命令>`。
  - `FenceJob` = `FencePlan` + `prepared`（外层是否已写完本机授权）+ `product_home`（产品私有区，容器 profile 的台账落点）+ `policy`（本进程的强制项与降级许可）；扁平成一个 JSON 传给守门进程。`product_home` 不叫 `home`：`FencePlan.home` 是 agent 的私有沙箱，两者不是一回事。
  - 命令作为**一个 argv** 交给守门进程，外层不拼 shell；模块工具的参数 JSON 经 stdin 送进工具进程，不进命令行；shell 的脚本自身经 stdin（Unix）或临时 `.ps1`（Windows）送达。
  - 守门进程用系统 shell 执行模块作者写的命令行（`sh -c` / `cmd /C`）——命令行是模块声明的一部分。shell 的启动命令由机制生成：Unix `sh -c "$(cat)"`；Windows PowerShell（pwsh 7 优先、5.1 回退，`-File` 临时 .ps1；脚本先设 `$ErrorActionPreference = 'Stop'`，未处理的 PowerShell 错误当场中止并以非 0 结束；脚本尾部 `if ($null -ne $LASTEXITCODE) { exit $LASTEXITCODE }` 把**最后一次原生命令**的码变成回执退出码（PowerShell 自己不会传播它），显式 `exit N` 优先）。
  - **容器里的程序名先相对化**（仅 Windows）：AppContainer 下**绝对路径**的程序会被拒（真机：同一文件用 `D:\…\x.exe` 报 Access denied，用 `..\x.exe` 正常——相对路径走 cwd 的目录句柄，不遍历卷根；非系统盘的卷根 DACL 要管理员才写得动）。执行前把首 token 的程序名解析成绝对路径、再改写成相对 cwd 的路径（同卷才改；裸名按容器里生效的 PATH / PATHEXT 解析；跨卷或解析不出则原样）。参数里的路径不动。
  - 环境白名单：外层 `env_clear`，只注入 `fence_env`（PATH / PATHEXT / 系统根 / LANG / TZ 等，`HOME` 与 `TEMP` 指向该 agent 私有沙箱）与模块隐秘字段；其它环境与凭据不进工具进程。
  - **围栏是固定并集，没有按权限项的分支**（派生在 `src/units/business/tools/domain/fence.rs`）：`FencePlan::for_call` 一律把私有沙箱（`home`，隐含一条写授予）+ 该模块私有区（写）与全部模块目录（只读子树）写进 `grants`，主机只读根是 `Grant::read`，`net` = 全局开关与档位求交；共享区主副本永不进围栏（只由宿主内的文件工具直读，容器里的进程要读先 `work_pull`）。`for_session` 用同一套并集作**会话 base**，撤权走会话租约，不用于按次派生。
  - **shell 的一次执行**：`run_shell` 走同一个守门进程；`timeout_secs` 是本次的（不沿用模块工具的 30 秒），到点或取消旗置位都杀整棵树；输出原样交回（模型可见的截断由工具层的回执预算统一施加），落点与本次耗时由工具层补成说明（码 `shell.place` / `shell.place.prep`；没跑成的调用不加落点行，但结构化事实仍记下落点与耗时）；每次调用都 `require_fs + require_net + allow_degraded=false`，装不上拒绝；Windows 的容器身份按 `会话租约 + agent + 计划指纹` 派生（同一计划复用 SID 与 ACL），授权按会话持有、会话改配 / 删除时回收；工具链与运行包是基础工具链基线，不参与按次授权。
  - **VM guest 的 per-call 断网**：guest helper 读策略里的 `net`，false 时在子进程前 `unshare(CLONE_NEWNET)`；失败以 111 fail-closed，不静默降级。helper 同时读 `timeout_secs` 并按它杀树；guest 输出按 `GuestRun.output_limit` 只留开头或开头 + 结尾。
  - **本机回环的一次性放行（仅 Windows）**：`net = Local { ports }` 时容器里 bind 的是 `127.0.0.2:<端口>`（容器与宿主共用一个回环栈，同端口同地址会撞 `WSAEADDRINUSE`），宿主按端口清单 dial `127.0.0.1:<端口>`，两侧用继承的匿名管道传帧、不走网络；端口清单与「产品自己的端口」都在**宿主侧**判，后者按「监听者 pid 落在本产品进程树里，或它跑的就是本产品这个可执行文件」判（`src/kernel/detail/container/windows/relay.rs`）。中继进程就是产品自己的可执行文件，它进容器要一条单文件只读授权（`FencePart::Relay`）。本档不许降级：中继起不来即以 `FENCE_FAILED` 拒绝，否则等于把回环全通给容器（`src/kernel/detail/confine/job.rs` 的 `local_relay` 判定）。
  - 收尾：stdout / stderr 分别读；模块工具超时 30 秒（`ProcTools::timeout`），shell 用本次 `timeout_secs`；到时连根杀树；输出原样交回——机制层不做模型可见的截断，16k 预算唯一在 `src/kernel/domain/budget.rs`。超时与机制失败记成回执说明的码；围栏裁决的文案归工具层，机制不硬编码。
  - 围栏装不上（本机不支持 / 容器起不来）：命中 `require_fs` / `require_net` 且未同意降级时，守门进程以 `FENCE_FAILED` **拒绝执行**；已同意降级才如实降级继续。Linux 的文件系统与网络两项**独立**判定，一项装不上不影响另一项。能力等级已在启动报告里说明。
  - Windows 外层未授权（`sandbox.policy.windows.write_acl=false`）：容器不装；启动门要求强制时只能本次降级 / 重设 / 退出（`src/main.rs`），否则按无围栏执行；全局网络开关不生效于容器，启动报告的「本次实际」已标出。
  - Windows 未授权时段先自检（`confine::verify`）：`EnvUnavailable` = 本机不支持，降级照跑；`Broken` = 自检说机制有效却装不上（我们写错了），**拒绝执行**，不按无围栏跑。
  - 固定退出码 `111`（`FENCE_FAILED`）用于：`FenceJob` JSON 非法、守门进程里工具命令起不来、工具被信号结束拿不到退出码，或全局策略要求强制而机制装不上（拒绝执行）。
  - 围栏生效且工具非零退出：回执只如实交退出码（`shell.exit` / `shell.signal` 说明），不补边界说明——拒绝发生在子进程内部，守门进程没有可靠信号（见 `tests/gaps.yaml` 的 `tools.fence-boundary-note-trigger`）。
- 平台差异：

| 平台 | 文件系统 | 出站网络 | 机制 |
| --- | --- | --- | --- |
| Linux | Landlock（运行基线 / 系统只读区只读，`/dev/null` 例外可写；基线与 guest 共用 `src/kernel/contracts/fence.rs` 的 `LINUX_READ_ONLY_BASELINE`，含 DNS 配置与 Debian / RHEL 的 CA 路径；其余按计划的授予；内核不支持或规则被拒时如实降级） | 按 `net`（真 = 不装网络过滤器；假 = seccomp 拒绝非 AF_UNIX / AF_NETLINK socket，TCP/UDP/raw 与回环都出不去） | 守门进程内装 Landlock 与 seccomp |
| macOS | seatbelt（`sandbox_init` 是私有且已弃用的 ABI；失效时如实降级） | 按 `net`（真 = 放行 `network-outbound`；假 = 拒绝 `network*`） | 守门进程内装 seatbelt；祖先元数据放行 `file-read-metadata` |
| Windows | AppContainer（外层 `prepare_fence` 把可达范围写进目录 ACL） | 按 `net`（真 = 授 `internetClient` 公网出站；假 = 不授） | 守门进程建容器 profile 并在容器内跑；作业对象管进程树与进程数上限 |
| 其它 | 无（如实降级） | 无 | 只有超时与整棵树终止 |

- 三平台同一口径：Linux 只读运行基线 + PATH 解析、Windows 按落点授权、macOS seatbelt；模块作者向结论见 [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 的「工具进程边界」与「路径模型」。

- **网络边界**：全局 `sandbox.policy.network` 是唯一开关。平台落点：Windows 授 `internetClient`（只放公网出站；不授 private network / loopback，也不做 loopback 豁免）；macOS `net=true` 放行 `network-outbound`（入站与绑定仍由 `deny default` 拒绝，回环出站可用），`net=false` 拒绝 `network*`；Linux `net=true` 不装网络过滤器（不限制入站 / 绑定，这是与 Windows / macOS 的平台差异），`net=false` 用 seccomp 拒绝非 AF_UNIX / AF_NETLINK socket。Windows 未授权 `windows.write_acl` 时容器不装，网络不生效（启动报告的「本次实际」会标出）。
- 进程树：Unix 起进程时开独立进程组，超时 / 退出时对负 pid 发 `SIGKILL`；Windows 把工具进程放进守门进程的作业对象（`KILL_ON_JOB_CLOSE`），守门进程一死整棵树随之消亡。
- **本机能力**与**本次实际**分开报（见 `src/main.rs` 的启动报告）：`sandbox.policy.windows.write_acl` 只决定 Windows 是否写目录 ACL；Linux / macOS 的围栏在守门进程里自足。所以「本次实际」在 Linux / macOS 上等于本机能力；Windows 未授权时 fs/net 为假，进程树、资源上限与环境白名单照旧生效。
- **进程级降级**：`FenceJob.policy` 携带 `require_fs` / `require_net` / `allow_degraded`；机制装不上时，`allow_degraded=false` 一律以 `FENCE_FAILED` 拒绝（所有平台一致，`src/kernel/detail/process.rs`），已同意降级才继续。降级只活在当前进程，不落盘。

## 三、长驻子进程的账与回收（`procs`）

- 账本（`data/state/processes.json`，`ProcessBook`）：
  - 一条登记：`pid` / `since`（unix 秒，只作呈现与先后判断）/ `purpose` / `owner` / `marker`（命令行里应出现的标记）/ `created`（身份）/ `guard_note`（父死子亡没挂上的原因，空 = 挂上了）。
  - 身份按平台用**能证明的判据**：Unix 是命令行里的 `marker`，Windows 是进程创建时间 `created`（`GetProcessTimes`）。
  - `purpose` 的当前取值：`guest`（QEMU）、`share:smb`、`share:virtiofs`、`resident`（常驻服务的守门进程）。
  - `owner`：QEMU 记租约；共享 helper 记落点（首个挂载 / guest）；常驻服务记 `resident/<模块>/<服务>`。
  - 打开时读不懂就当空账；父目录不存在就补建。落盘用同目录临时文件 + rename。
  - 同一 pid 只留最新一条；`list` 按 `(since, pid)` 升序。
- 施工面（`src/kernel/detail/procs/spawn.rs`）：单元只经 `kernel::api::process::{Spawn, Stream, spawn, Daemon}` 起长驻子进程，`spawn` 一次办完五件事——进程组长与父死子亡、长命线程上 spawn、stderr 常驻抽干（并逐行 UTF-8 宽容解码）、`attach` 接线与读身份、登记进账本；命令行怎么拼、就绪判据、优雅停机与重启策略仍留给单元。
  - `Spawn` 的形状：`program` / `args` / `cwd` / `clear_env` / `env`（逐条给）/ 三路流（`Null` / `Inherit` / `Piped`）/ `tail`（留几行 stderr）/ `echo_stderr`（每行转写：进日志、上控制台）/ 账上身份（`purpose` / `owner` / `marker`）/ `book`。
  - 拿到的 `Daemon`：`try_wait` / `wait_exit` / `stderr_tail` / `take_stdin` / `take_stdout` / `stop`；**丢掉即收掉**（`Drop` 先礼后兵，按进程组或整棵树），收场同时销账。`stop` 幂等。
  - stderr 只有 `Piped` 才抽干；抽干是常驻线程（管道写满会把子进程卡住）。尾行容量有界（默认 20 行），只留尾部，长跑的进程吃不掉内存。
- 父死子亡（`procs::guard` / `bind` / `watch`，接线走 `attach`——三者都是 `pub(crate)` 的内部零件，只给 `spawn.rs` 用）：
  - Unix 起进程前开独立进程组（`process_group(0)`），收树不牵连核心。
  - Linux：`pre_exec` 里设 `PR_SET_PDEATHSIG=SIGKILL`，核心一死由内核收掉子进程。
  - macOS：`procs::watch` 另起本程序的守门进程 `--proc-watchdog <核心 pid> <子进程 pid>`；它用 kqueue 的 `EVFILT_PROC` / `NOTE_EXIT` 同时盯核心与子进程，核心先退就按进程组 `SIGKILL` 子进程。守门进程清空环境、独立进程组、不挂 PDEATHSIG；起不来如实返回原因，不阻断启动。
  - Windows：`procs::bind` 把子进程绑进一个 `KILL_ON_JOB_CLOSE` 作业对象；核心一死，作业关闭，子进程跟着死。
  - `procs::attach` 是 `spawn.rs` 里起进程之后唯一的接线点（`bind` + `watch`，并读一次身份）：挂不上只打一条 `[进程]` 并把原因交回调用方，**不阻断启动**。调用方把它记进账的 `guard_note`——`--doctor` 与界面据此把该条标成「核心被强杀会留下孤儿」，下次启动对账再按身份收掉它。
  - Unix 上 `procs::spawn_raw` 统一切到常驻的 `solomni-proc-launcher` 线程上 spawn：`PR_SET_PDEATHSIG` 绑的是**发起 spawn 的线程**；在 Web 每请求一线程或短命工作线程上直接 spawn，线程一退就会把 QEMU / 共享 helper / 常驻服务守门进程一并杀掉。
- 启动对账（`ProcessBook::reconcile`，组合根在会话开工前跑一次）：
  - 进程已不在（`alive` 为假）：删掉条目。
  - Windows：读该 pid 的创建时间，与账上 `created` 对得上才 `taskkill /PID <pid> /T /F` 收掉；对不上按 pid 已被复用作废；账上没有 `created`（更早的账）只报不动手。
  - Unix：读该 pid 的命令行；含该条 `marker` 才 `terminate`（按进程组先 TERM，最多等 3 秒，再 KILL——守门进程拉起的服务跟着收）；不含就按 pid 已被复用作废条目。
  - 报告三项：`dropped` / `killed` / `alive`；有动静才打印一行。
- 存活探测与终止：
  - 存活探测只有一处实现：`src/kernel/detail/procs.rs` 的 `presence(pid) -> Presence`（`Live` / `Dead` / `Unknown`，`Unknown` 只表示权限不足探不到）。Unix 用 `kill(pid, 0)` 取 `ESRCH` 判死；Windows 用 `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `GetExitCodeProcess`，句柄打不开且错误码是 `ERROR_ACCESS_DENIED` 记 `Unknown`。`alive` 只是 `presence == Live` 的简写。
  - **三态由调用方解释**，因为两边的保守方向相反：进程账要回收，探不到按死处理（账上留死记录比误判活着更糟）；视图盘回收要留住正在用的盘符，探不到按活着处理（宁可漏回收也不误撤映射）。
  - 归属证据（创建时刻 / 命令行标记）才是身份判据；`presence` 只回答「这个 pid 还在不在」，不用来认亲。
  - 终止：Unix 按进程组先 TERM 再 KILL（不是组长的就发单 pid）；Windows 用 `taskkill /PID <pid> /T /F`。
- 接线点：
  - QEMU 引擎、SMB helper、virtiofsd 在起进程处只调一次 `spawn(Spawn{…})`（各单元自己的 argv、就绪判据与停机语义仍在原地）；销毁时 `Daemon::stop`，或直接丢掉句柄。
  - 常驻服务的守门进程（`src/kernel/detail/session.rs` 的 `ProcessSessions::open`）走同一处施工面（`purpose = resident`），登记进同一份账本：会话收掉时注销，核心被强杀留下的孤儿由启动对账按身份收掉。
  - 观测：`--doctor` 的 `processes` 与 `GET /api/processes` 读同一份账本，每条带 `orphanRisk` / `guardNote`；覆盖 QEMU、共享 helper 与常驻服务守门进程。

## 四、运行日志（`Log` 端口）

- `src/kernel/ports.rs` 定义 `Log`（`info` / `warn` / `error` / `purge`），业务只**调用**；文件、时间戳与目录机制在 `src/kernel/detail/file_log.rs`。
- **旧日志只经 `purge` 删**：清理整类日志时由 `Log::purge(name)` 删，本次运行那份由 `FileLog` 自己拒删（比较文件名）；`Log` 的默认实现不删，无声日志与测试替身因此没有盘可动。业务单元不自己 `remove_file` 日志。
- **唯一实例**：组合根创建一个 `FileLog` 并注入协调业务与呈现层；测试用 `NoopLog`。
- **按运行分文件**：每次运行在数据根 `data/logs/` 下按时间戳建一个 `.log` 文件；`data/logs/` 不入库。一次运行的因果链在同一份文件里，排查不跨文件拼。
- **时间戳取本机时区**：文件名与行首时刻都由 `clock` 机制给出（`src/kernel/detail/clock.rs` 的 `local_now`），不再按固定偏移折算。
- **关键节点必须埋点**：通道降级、HTTP 失败、会话动作失败、装配失败、工具执行异常（端口在 `src/kernel/ports.rs`，机制在 `src/kernel/detail/file_log.rs`）。出问题时**看日志定因**，不靠推理猜。

## 五、机制分层（`mechanisms.yaml`）

- 按**机制**归层，一个机制管一种机制：`contracts/<名>`（共享形状）、`domain/<名>`（纯逻辑）、`detail/<名>`（实现）、`ports`（全项目共享的出站面）。表在 `docs/kernel/mechanisms.yaml`，判据在 `tests/gates/structure/kernel-mechanisms.js`。
- 三条硬判：盘上每个机制都要归层、表里的机制也要在盘上；依赖边只许指向严格更低层（**同层互依赖判红**）；声明层号 == 按现有依赖重算的层号。第四条是环：机制之间成环时只报环路径，不拿层号糊过去。
- 当前 38 个机制、12 层，读到顶的顺序是 `detail/launch`（5）→ `detail/container`（6）→ `detail/acl`（7）→ `detail/ledger`（8）→ `detail/confine`（9）→ `detail/process`（10）→ `detail/session`（11）：越往上越是装配别人的机制。
- 围栏这一族按职责切成五个机制，每个只依赖下一层：`launch`（命令怎么起：PATH 解析、环境白名单、程序名改写）、`container`（容器身份与容器内拉起）、`acl`（逐路径改 DACL、读完整性标签）、`ledger`（授权台账与按条回收）、`confine`（编排：授权→执行→收尾，平台后端的策略判据）。
- 动过一条边就 `node tests/gates/structure/kernel-mechanisms.js --write` 重算全表；不想抬层就消掉那条边。层级理由写在 diff 里，不在散文里。
- 粒度区别：单元（`layers.yaml`）按业务用例切、可以互相调用对方的 `api`；机制（本表）没有业务语义、只许单向。机制不因调用方多就升格成单元。
- 第五条判据：内核内部只走机制路径，不经 `api` 面（见「六、对外面」）。

## 六、对外面（`api/`）

- 一个文件一个**对外概念**：`src/kernel/api.rs` 只声明模块，条目在各面文件里重导出。面是单元与装配层取形状与机制的唯一入口（`crate::kernel::api::<面>::<条目>`）。
- **内核内部不经此面**：机制之间直取所属机制（`crate::kernel::contracts|domain|detail::<机制>::…`）。判据是 `tests/gates/structure/kernel-mechanisms.js` 的第五条——`src/kernel` 下（除 `api/` 面自身）出现 `kernel::api` 就判红。
  - 理由一：面是给外面的形状，不是内部捷径；内部绕道面会让面与实现互相牵制（改一个条目的归属要连着改面）。
  - 理由二：绕道 `api` 的依赖在机制表里是**隐形**的（`api` 不是机制，边扫不到）；直取机制后机制边才如实，层号才是事实。
- 同名机制合成一面：`contracts/fence` 与 `domain/fence` 都在 `api::fence`（描述符与沙箱策略是同一个对外概念）；模块型条目整模块重导出，保住 `api::layout::home(…)` / `api::i18n::render(…)` 的书写。
- 新增一个对外条目：先落进所属机制，再在对应面里重导出；**新开一个面**要同步 `docs/kernel/unit-map.md` 与本页。眼下没有外部调用方的条目用 `#[allow(unused_imports)]` 显式放行并写理由，不从出口删掉——出口少一个，契约就悄悄漂了。

## 七、IO 原语（`detail/io`）

- **只有一份实现，不做端口**：出站网络与进程环境落在 `src/kernel/detail/io/`，单元经 `kernel::api::net` / `kernel::api::env` 取用；平台差异收在 `net/backend/{unix,windows}.rs`（`src/kernel/detail/io/net/backend/`）。判据与理由见 [ports.md](../architecture/ports.md) 的「IO 原语不画端口」。
- **出站**：ureq 与 reqwest 两条通道共用一份平台 TLS 决策（`src/kernel/detail/io/net/backend.rs` 的三个出口），连接 / 总超时预算在 `src/kernel/detail/io/net/outbound.rs`；内核不引 reqwest / tokio，也不判 4xx/5xx 与重试（那是供应商语义，留单元）。
- **入站回环**：`src/kernel/detail/io/net/loopback.rs` 的 `serve(port, max_body, handler)` 只绑 `127.0.0.1`，每请求一线程，请求体超过上限或不是 UTF-8 就由它直接回 400（handler 见不到坏 body）；交出 `Loopback`（`port` / `url` / `stop`，丢掉即收）。呈现层的转录中心与虚拟机的 cloud-init 种子服务都走它，`tiny_http` 的类型不出这一层。
- **回环 TCP**：`src/kernel/detail/io/net/tcp.rs` 的 `reserve_port`（先占后放，只压小碰撞面、不作保留）、`dial`（唯一写死回环客户端地址处）、`await_loopback`（就绪轮询 + `gone` 回调，让调用方把「对端已死」说成自己的话）。
- **进程环境**：`src/kernel/detail/io/env.rs` 是唯一读取面——`var` / `os` 两个原语 + 具名宿主事实（PATH、PATHEXT、SystemRoot、LOCALAPPDATA、可执行文件、临时目录）。`std::env::` 只许在它、`src/main.rs`、`src/entry/` 与 `src/presentation/`（终端能力探测）出现，其余经机制——门禁 `structure/env-surface` 判。
- **内核不认识产品**：IO 层只提供原语，产品名与它的 argv / 配置文件名留在单元（门禁 `structure/kernel-products`）。

## 八、文件事务与宿主程序

- **写盘只经 `kernel::api::fs`**：`write` / `write_private` / `append` / `stream_into` / `transact_with` 把「按路径排序取锁（进程内互斥 + 跨进程 `lock_within`）→ 同目录暂存 → `Expect` 在锁内比对 → 一次发布 → 失败整体回滚」一次做完（`src/kernel/detail/fs.rs`）。旧版本比对在锁里，所以读后写门禁与写入之间没有 TOCTOU：期望的旧版本是事务形状的一部分，不是调用方自己再查一遍。
- **锁目录由组合根装一次**：装配期调 `install_lock_space(LockSpace::new(layout::locks(&root)))`（`src/main.rs`）；调用方不拼锁名、不猜锁文件放哪（键由落点身份派生，同一物理对象同键）。显式传入 `Some(&space)` 仍优先，测试据此换等待上限。
- **宿主程序只经 `kernel::api::process::run_to_end`**（`Run` / `Done`）：等到底、超时杀整棵树、输出保留字节、可送 stdin；单元里「跑一个外部程序到结束」都走它。会话内的执行不走这里——那由 `sandbox` 的执行端口按策略开句柄。
- **策略不进内核**：能不能写、写什么内容归 `workbench`；版本语义与读后写门禁归 `versions`；围栏与可达域归 `detail/confine`。内核只答「怎么安全地写、怎么安全地跑」。
- 两个面各有一条门禁：`structure/fs-surface`（除 `src/kernel/` 外不许 `std::fs` 变更原语与 `File::create` / `OpenOptions::new`）、`structure/process-surface`（除 `src/kernel/` 外不许 `std::process::Command::new`）。判据在 `tests/gates/structure/`，清单见 [boundaries.md](../architecture/boundaries.md)。

## 改动本单元时必须同步

- 共享形状（R6）一改，所有使用者同改；路径拼接规则见 `docs/architecture/rust-dev.md` 的「跨平台机制」（禁止把分隔符写进字符串）。
- 平台专属代码本地不编译——`FencePlan` 字面量必须写全字段（跨平台字面量门禁，`src/kernel/contracts/fence.rs`）；改围栏 → `tests/<平台>/` 探针。
- 围栏的写后核对与回滚在 `src/kernel/detail/acl/windows/verify.rs`、台账在 `src/kernel/detail/ledger/windows/`：改落点形状（`GrantTarget`，`src/kernel/detail/acl/windows/target.rs`）或台账字段时，`prepare_fence` / `release_fence` / `clean` / `clean_all` / `sweep_orphan_aces` 与按条处置（`catalog` / `restore_one` / `revoke_grant` / `remove_profile_one`）要一起改。
- `clean` 与 `clean_all` 的分工：前者默认只撤按次 / 会话授权与 profile，保留 `scope=machine` 条目（测试每轮收尾不重写祖先 ACE）；后者连机器级一起撤，是 `--fence-clean` 的入口。带机器级授权的路径不能按快照整体还原（会把机器级 ACE 一起抹掉），这些路径上的按次 ACE 走逐条精确撤销。
- 改**归属字段**（含会话租约 `Sandbox.session` → `FencePlan.lease` → `Owner.lease`）或启动对账时，`src/kernel/detail/ledger/windows/liveness.rs` 的 `current_owner` / `owners_state`、`src/kernel/detail/ledger/windows/reconcile.rs` 的 `reconcile` 与 `src/kernel/detail/ledger/windows/release.rs` 的 `release_fence` / `clean` / `clean_all` 要一起改；启动对账的接线在 `src/main.rs`。
- ACE 的读法只有一处：`src/kernel/detail/acl/windows.rs` 的 `ace_parts` 按 ACE 头算 SID 偏移（标准 ACE 与回调 ACE 从第 8 字节起，对象 ACE 再加 Flags(4) 与在场的 GUID）。改覆盖的 ACE 类型或身份形状时，写后核对、诊断转储与 `src/kernel/detail/confine/windows/tests.rs` 的对象 ACE 往返探针要一起看。
- 容器里**列目录**的判据：同一个授权叶子上，cmd 的 `dir` 与 PowerShell 的 `Get-ChildItem` 会被拒；cmd 的 `for` 枚举与 python 的 `os.listdir` / `os.scandir` 正常。所以「列自己的产物」用 `for` 枚举钉住（`src/kernel/detail/confine/windows/tests.rs` 的 `container_roundtrip_sees_leaf_but_not_parent_content` 第四步），`dir` 不是可依赖的列举手段。
- **解释器按真身授权，容器 PATH 先换真身**：命令名经目录重解析点（符号链接 / 联接）进来时，`fence_env` 把容器 PATH 里的绝对目录项逐层换成真身（`src/kernel/detail/launch.rs` 的 `canonical_path_var`），`interpreter_roots_for_exe` 只授真身安装根——容器不再按 junction 原路径搜索，链接对象因此不需要 DACL（此前靠 PATH 穿链接、只授首尾会漏掉中间联接；真机 nvm 的 `.nodejs` → `.link\nodejs` → 版本目录曾报 `node` 找不到）。不往链接或 `C:\hostedtoolcache` / `C:\Program Files` 这类大目录写 DACL：前者会被写后核对判成丢项，后者会触发整树继承重算（CI 上曾把 Python 工具调用拖过 120 秒）。相对项与取不到真身的项原样保留。预检与执行共用这份清单。
- **预检即终局**：解释器根先过 `interpreter_dir_usable`（已有 AAPP 只读、属主、或 `AccessCheck` 判出 `WRITE_DAC`）；不可用就直接记成必要落点缺失，**不尝试写**（`src/kernel/detail/container/windows.rs`）。
- 命令里的解释器有**基线**：`node <文件>` 的命令由围栏注入 `NODE_OPTIONS=--preserve-symlinks --preserve-symlinks-main`（`src/kernel/detail/container/windows.rs`）——node 的 `fs.realpathSync` 会 lstat 卷根与祖先前缀，而这两类落点按设计不在可达范围；少了它，进程在脚本执行前就 EPERM 死。只在 Windows 注入（Landlock 不管读属性、seatbelt 已给祖先放行 `file-read-metadata`）；判据与 `interpreter_dirs` 共用同一份 PATH 解析（`command_programs_in`）。
- **真机测试（`--fence-live`）**：`unwritable_interpreter_dir_never_silently_runs_unfenced`（`ProcTools` 的用例）断言三条：选「跑一次」才跑且回执如实标为无围栏；选「放弃」不执行；没有可回答的前端也不执行。宿主解释器的真身可授权时如实 env-skip（构造不出「授不上」这一态）；「写失败不报回滚、不撤预存 ACE」的契约由注入用例 `failed_write_keeps_preexisting_ace_and_never_says_rollback` 覆盖。
- 改 `procs` 的登记字段或回收判据时，本页的「三、长驻子进程的账与回收」、`src/main.rs` 的启动对账、`--doctor` 的 `processes` 字段与 `GET /api/processes` 的响应要一起看。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`mechanisms.yaml`](mechanisms.yaml) | 机制与机制之间的层号（T0 双向比对，动过边就跑 `--write`） |