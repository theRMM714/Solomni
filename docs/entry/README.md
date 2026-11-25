# entry（根：启动、组合根与探针）

> 定位：根——启动程序、组合根、内部协议探针、围栏守门进程与长驻子进程守门进程；依赖图最外层，不是业务层。
> 效力：规范｜核对：2026-11-25｜依据：`src/main.rs`、`docs/entry/layers.yaml`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：根共用机制——产品根规范化（`src/entry/root.rs`）与启动期基础工具链 PATH（`src/entry/toolchains.rs`）。
- 管：组合根（`src/main.rs`）——跑启动对账、把入站面交给 CLI 或 Web；适配器与入站面统一在 `src/entry/assembly.rs` 装配。
- 管：内部协议探针与隐藏模式（`src/entry/probes/`：围栏环境与机制验证、出站 HTTPS、解归档、路由目录）。
- 管：两个守门模式（`src/guard.rs`）——围栏 `--fence-run`、长驻子进程 `--proc-watchdog`，以及围栏台账的按条处置。
- 不管：没有业务语义；产品命令（`--doctor` / `--provision` / `--gc` / `--storage` / `--work-gc` / `--build`）归启动相命令面，语义在 `system` 单元（见 [docs/system](../system/README.md)）。
- 不管：任何单元都不许依赖本单元（`run-tests.js` 的依赖方向门禁）。

## 入站契约与状态归属

- 入站面：本单元对单元不提供入站契约；`src/main.rs` 装配核心后把面袋 `presentation::faces::Faces` 交给呈现层。
- 状态归属：本单元不持有业务状态；启动期读写的围栏台账与进程账分别归 `kernel` 的 `confine` 与 `procs`。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——依赖图最底层或纯领域）
- 谁在用我：（无）

## 一、组合根（`src/main.rs`）

- 先处理**不需要装配核心**的模式：`--proc-watchdog`、`--fence-run`、`--https-check`、`--unpack`、`--print-fence-env`、`--fence-verify`、`--print-routes`。
- 再解析全局旗语（`src/presentation/cli/argv.rs`）：`--root <路径>` / `--web` / `--port <n>` / `--plain` / `--no-anim` / `--no-workspace` / `-h, --help` / `-v, --version`；写错直接报错退出 2，`--help` 与 `--version` 不装配核心。`--setup [模式]` 不在这里——它是启动相命令，与其它维护命令一样由命令表识别、由启动相执行。
- 产品根：`--root <路径>`，缺省 `.`；相对路径与当前目录做纯词法拼接，取不到当前目录才退回 `canonicalize`（Windows 上剥掉 `\\?\` 前缀）。
- **便携维护**（`--setup [模式]`）是启动相命令，命令行与转录中心提示符两条入口一个实现：工具链就位后由 `src/presentation/cli/boot.rs` 把模式名翻成 `SetupMode` 交 `system` 单元，写入实现（注册表、shim、ACL）在 `src/units/system/detail/setup.rs`，启动期的只读对账在 `src/entry/portable.rs`（只经 `SetupOps` 端口取缺项与预授，见 [docs/system](../system/README.md)）。模式：`path` 生成 `bin/solomni.cmd` 并把产品根的 `bin` 追加进用户 PATH，`unset` / `undo` 摘除或按备份还原，`status` 打印 JSON（缺省模式），`fence` 做围栏机器级预授并退役旧的全应用包机器级授权，预授成功后把授权记进设置（`sandbox.policy.windows.write_acl: true`，经登记处校验后落盘；写失败只把回执降级成一行 `[便携] 未写 …：原因`，预授本身仍算成功），`all` = path + fence。这些模式不读登记处；**启动期的诊断（工具链 / 围栏 / 进程 / 便携 / 沙箱授权）一律走 stderr**，stdout 只留给命令结果——所以 `--setup status --json` 与转录相的一次性命令（`--json modules`、模块手册与工具、`work` …）的 stdout 都能整段进管道与 `JSON.parse`。`--setup=path` 写出的 `bin/solomni.cmd` 把 `--root` 固定到**安装根**（shim 自身所在目录的上一级），所以经 shim 调用时产品根不再取决于当前目录；直接运行 `solomni.exe` 且未给 `--root` 时，根仍按上一条取当前目录。
- 建成核心、进主循环之前分配**视图盘**（仅 Windows）：产品根映射成本实例独占的盘符并登记当前 pid 与进程创建时刻（pid 会被系统复用，创建时刻对不上就不算自己的），shell 这类 cwd == 私有沙箱的调用里，容器 cwd 与模型看到的路径用视图写法（模块工具沿用宿主 cwd，不参与视图盘收敛），宿主真实路径由路径码翻译；正常退出释放映射与登记；Ctrl+C / 关窗 / 注销 / 关机这类控制台终止由分配时装上的控制台钩子先撤（映射与登记一起，进程随后照常终止）；崩溃 / 被强杀来不及跑的残留——盘符映射是登录会话级的，进程死了也还在——由**每次启动**的清扫把记录与映射一起摘掉，扫完没有条目就连记录文件一起删，不留空数组。**只有真正跑会话/服务/工具的主流程才分配**——一次性命令（命令表里 `runtime: none`：`modules`、手册、`work` …）、启动相命令、维护旗与探针都是早退路径（`std::process::exit` 不走 Drop），分配了必然泄漏盘符，它们只调用「只清扫不分配」的入口。`SOLOMNI_VIEW_DRIVE=0` 是排障用的逃生口（关闭后行为与旧版一致）；分配失败只打一行 `[围栏]` 回退真实路径，不阻断启动。
- 根解析后立即应用**基础工具链**：读 `.tools/toolchains/active.json`（由 provision 的 `kind: toolchain` 写入），把项目内 `bin` 前置到 PATH；目标不符 / 路径不安全 / bin 不在场只报告，不阻断启动。**每次启动**都打印基础 profile 的项目内 / 宿主回退（Windows 探围栏能否授权）/ 缺失与补全指令；之后 `--doctor`、`--build`、会话工具与常驻服务都看这份生效 PATH。
- 产品维护命令不由入口分派：工具链就位后，`flags.command` 交给启动相（`src/presentation/cli/boot.rs`，经 `system` 单元执行）。围栏台账系列（`--fence-reconcile` / `--fence-ledger` / `--fence-restore` / `--fence-revoke <SID> <路径>` / `--fence-profile-rm` / `--fence-clean`）仍留在入口：只需要产品根，不装配核心；撤授权按 SID + 路径两个参数，其它 arity 打用法退 2。
- 装配期失败一律打印 `[装配失败]` 并以 1 退出（`src/main.rs`）：登记处四份 yaml 读不了、提示词册缺文件 / 缺键、工具总表与机制面不自洽、模块清单装配失败、隐秘字段加载失败、核心执行线程起不来；日志文件起不来只降级到 stderr（进程继续）。
- **启动对账**在会话开工前跑两份：围栏授权台账（`confine::reconcile`）与长驻子进程账（`ProcessBook::reconcile`，`data/state/processes.json`）。
  - 围栏台账只回收归属明确已死的授权。
  - 进程账清掉已不在的登记，并按**能证明的身份**收掉上次留下的孤儿：Unix 看命令行标记，Windows 看进程创建时间；认不出身份的只报不动手（不误杀别人的进程）。机制见 [kernel 的「长驻子进程的账与回收」](../kernel/README.md)。
- **围栏写权限**：`SOLOMNI_FENCE_WRITE=1/0` 优先，否则读设置里的 `sandbox.policy.windows.write_acl`；这一份值在**预检前**只读一次，预检、启动报告与工具进程共用（Linux / macOS 的围栏在守门进程里自足）。启动报告把「本机能力」与「本次实际」分开打印：Unix 的「本次实际」等于本机能力；Windows 未授权时 fs/net 为假（容器围栏不装），进程树仍生效。
- **启动门**：解析 argv 只决定形态；要不要过工具链 / 构建 / 设置 / 围栏预检只看命令声明的 `runtime`（`systools/commands.yaml`）——`runtime: services`（默认 CLI、`--web`、会话、模块工具调用）要过，`runtime: none` 的一次性命令（`modules`、手册、`work` …）不看围栏授权，在常驻服务与视图盘之前跑完（判定在 `src/presentation/cli/shell.rs` 的 `needs_runtime`，门与分派共用这一份）。围栏要求没满足时只能本次降级 / 重设 / 退出（`src/main.rs`）；没有可回答的前端（非 TTY）时 stderr 逐项列出缺项（`[沙箱] <名字>：<原因>`）与三条能整行复制的出路（`SOLOMNI_FENCE_WRITE=1 <原命令>` / 设置里写 `write_acl: true` / 把 `enforce` 降为 `best_effort`），本机能力缺失的项只给最后一条，然后以 1 退出；`on_unavailable=refuse` 且不重设 = 退出，不会起 CLI 或 Web。`--web` 不是绕过，门通过后才会 `serve_web`。
- **首次启动**（没有 `data/.home/settings.yaml`）：Windows 上先问一次是否授权写 ACL；选是写盘并以退出码 75 整体重启，选否不写盘、进入启动门。授权成功后立刻跑一次机器级预授（同一套 ensure、逐条显示进度）；之后的启动只做 O(深度) 只读对账，缺项打一条 `[便携]` 提示并指向 `--setup=fence`，不静默慢写、不阻断启动。重设只修能修的那一项：未授权写授权；本机能力缺失则明确问是否把 `enforce` 降为 `best_effort`。手动预授 `--setup=fence` 预授成功后会顺手把 `sandbox.policy.windows.write_acl: true` 记进设置（下次启动不再被授权轴拦住）。
- **配置一致性**：Windows 上 `require` 文件系统 / 网络围栏 + `write_acl: false` 保存时拒绝、已存在的 `settings.yaml` 加载时报 `配置非法`；首启没有文件不拦（交给引导）。
- **全局沙箱策略**：`sandbox.policy` 是进程级启动事实；保存后整体重启。默认 `require`，机制不可用时启动门按 `on_unavailable` 询问本进程降级（不落盘）、重设或退出；`SOLOMNI_SANDBOX_DEGRADED=1/0` 只供测试 / CI 注入。
- **常驻服务**按有效启停拉起（`ResidentOps::start_enabled`，进程级租约，不属于任何会话）。
- **核心**搬到自己的执行线程（`Handle::spawn`）；呈现层只拿 `Faces`。
- `--probe-tools` / `--probe-replay` 在核心装配后执行：前者把**确定**结论写回 `models.yaml`，后者只报事实、不改登记处。

## 二、内部协议与探针

- 约定：多数模式恒退出 0，判定归调用方；需要区分的按「输出 / 退出码」列。产品命令（含它们的旗语、确认门槛与单条手册）不在本节——见 `systools/commands.yaml` 与 [docs/system](../system/README.md)。

| 模式 | 作用 | 输出 / 退出码 |
| --- | --- | --- |
| `--https-check <url>` | 用产品自己的出站链路打一次最小 HTTPS 请求 | `ok` / `no-net` / `env-tls` / `tls-fail\|fail`；0，缺 URL 2 |
| `--unpack <archive> <dest>` | 解归档：tar / tar.gz / tgz / tar.zst / tzst / tar.xz / txz / zip | 0 / 1 失败 / 2 用法错 |
| `--print-routes` | HTTP 路由目录的唯一定义（见 [contracts.md](../presentation/contracts.md)） | JSON；0 |
| `--print-fence-env <FenceJob JSON> -- <命令>` | 逐行输出运行期给工具进程的环境白名单 | `KEY=VALUE`；0，围栏参数非法 111 |
| `--fence-verify <FenceJob JSON> -- <命令>` | 只做机制验证，不装围栏、不写权限项 | `enforced` / `env-unavailable …` / `broken …`；0，参数非法 111 |
| `--probe-tools <模型 id>` | 实测一条通道是否支持原生工具调用，把**确定**结论写回 `models.yaml` | 三态文本；0 / 1 失败 / 2 用法错 |
| `--probe-replay <模型 id>` | 只报回放形状的事实，不改登记处 | 逐形状文本；0 / 1 失败 / 2 用法错 |

- 围栏台账（隐藏模式，用户经文档知道）：

| 模式 | 作用 | 退出码 |
| --- | --- | --- |
| `--fence-reconcile` | 按归属对账回收陈旧授权（启动期也自动跑一次） | 0；有错误 1 |
| `--fence-ledger [--human]` | 列台账（默认 JSON；`--human` 一行一句） | 0；序列化失败 1 |
| `--fence-restore <路径>` | 按路径还原一条快照 | 0 / 1 失败 / 2 用法错 |
| `--fence-revoke <SID> <路径>` | 按 SID + 路径撤一条授权（台账外的残留也照撤） | 0 / 1 / 2 |
| `--fence-profile-rm <profile 名>` | 按名删一个容器 profile | 0 / 1 / 2 |
| `--fence-clean` | 整体收尾：台账 + 孤儿 ACE + 容器 profile + 视图盘登记与残留映射 | 0 / 1 失败 |

- 内部协议（模块作者与用户不接触）：
  - `--fence-run <FenceJob JSON> -- <命令>`：围栏守门进程，由 `ProcessRunner` / `SessionHost` 拉起；退出码即工具退出码。`FenceJob` 非法或守门进程里工具命令起不来时用固定码 `111`（`FENCE_FAILED`）。协议细则见 [kernel 的「工具进程与守门协议」](../kernel/README.md)。
  - `--proc-watchdog <核心 pid> <子进程 pid>`：macOS 的父死子亡守门进程，由 `procs::watch` 拉起；只在 macOS 起作用，其它平台如实报错并退出 2。机制见 [kernel 的「长驻子进程的账与回收」](../kernel/README.md)。
- **进程账的呈现**：
  - `--doctor` 的 `processes` 字段列出账上每一条（`pid` / `alive` / `purpose` / `owner` / `since` / `orphanRisk` / `guardNote`）；`orphanRisk` 为真 = 父死子亡没挂上（核心被强杀会留下它），`guardNote` 是原因。覆盖范围见 [kernel 的「长驻子进程的账与回收」](../kernel/README.md)。
  - Web 经 `GET /api/processes` 读同一份（`Provision::processes`）；响应形状以 [contracts.md](../presentation/contracts.md) 为准。

## 三、入口内部分层（`layers.yaml`）

- 唯一权威：[`layers.yaml`](layers.yaml)，一行一条「模块: 层号」；模块名＝`src/entry` 下的文件名/目录名（目录算一个模块），编排根单列成 `main`（`src/main.rs`）。`src/entry.rs` 只是 mod 声明，不参与分层——混进代码就失败。
- 四段（数字大＝更靠上、更晚接进流程）：**值层**（`root` / `toolchains` / `probes`：只读事实与纯值）→ **判定层**（`startup` / `portable`：只决定能不能开工）→ **装配层**（`assembly` / `web` / `args`：把端口接成入站面、把面袋交给呈现层）→ **编排层**（`main`：只串流程）。
- 判据（T0 的 `structure/entry-tiers`，与 [kernel 机制表](../kernel/README.md) 同构）：① 盘上的模块必须在表里、表里的模块必须在盘上；② 依赖边的目标段必须**严格更低**——同段互相依赖与向上依赖都失败，因此不可能成环；③ 声明段号必须压住它的全部依赖（压不住失败）。
- 段是**启动阶段职责**，不是依赖深度的副作用：比依赖深度松不算错（例如 `startup` 没有入口内依赖，声明第 1 段是为了让它排在值层之后、装配之前）。这一点与 kernel 机制表不同——那张表的层号是按依赖重算的事实缓存，写宽就失败。
- 不在本表：`src/guard.rs`（围栏守门 `--fence-run` 与长驻子进程守门 `--proc-watchdog` 的第二个程序入口，由 `src/main.rs` 分发）——它们是入口拉起的**独立子进程**，不是启动流程的一段，依赖方向由 `tests/gates/structure/dependency.js` 的跨单元规则管。
- 只回执不判红：扇出（数字说明一处装配了多少模块）。
- 检查与重算提示：`node dev.js run -- node tests/gates/structure/entry-tiers.js`。

## 改动本单元时必须同步

- 便携维护（`--setup`）的行为一改 → 同步 `docs/system/`（实现在 `system` 单元）、本页的启动期对账说明（`src/entry/portable.rs`）、[docs/permission/README.md](../permission/README.md) 与 `tests/gaps.yaml` 的平台缺口。
- 新增端口 / 适配器 → 改 `src/entry/assembly.rs` 装配；新增探针或隐藏模式 → 同步本页、`docs/entry/unit-map.md` 与 [docs/testing/execution-ci.md](../testing/execution-ci.md)。
- 新增或改产品命令 → 改 `systools/commands.yaml`；操作数解析在 `src/units/tools/domain/op.rs`（声明派生），执行分派在 `src/presentation/cli/boot.rs`（启动相，经 `system` 能力）与 `src/presentation/cli.rs`（转录相），一致性由 `src/tests/commands.rs` 锁死。
- 路由目录的机器比对在 `src/tests/routes.rs` ↔ [docs/presentation/contracts.md](../presentation/contracts.md)。
- 启动对账或进程账字段一改 → 同步 [kernel 的「长驻子进程的账与回收」](../kernel/README.md) 与本页的呈现说明。
- 新增 / 删除 / 移动 `src/entry/` 下的文件或改内部依赖 → 同步 [`layers.yaml`](layers.yaml) 与 [`unit-map.md`](unit-map.md)：前者按模块归段（同段不许互相依赖），后者逐文件对账。
- 测试缺口：本单元是组合根、不是业务单元，没有独立的缺口账；缺口记在 `tests/gaps.yaml`；格式见 [docs/testing/gaps-acceptance.md](../testing/gaps-acceptance.md) 的「缺口账」。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`layers.yaml`](layers.yaml) | 入口内部的段号（唯一权威；T0 的 `structure/entry-tiers` 双向比对） |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |