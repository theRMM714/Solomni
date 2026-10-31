# 执行入口、报告与 CI

> 本文是**测试执行入口、报告状态、成功标记与 CI 流程的唯一权威**。
> 质量门禁见 [quality-isolation.md](quality-isolation.md)，验收清单见 [gaps-acceptance.md](gaps-acceptance.md)。

## 一、执行入口与报告

### 本机开发回路

改一层、验一层用 [dev.js](../../dev.js)（它把固定子命令映射到项目内的 cargo，**不要直接调 `cargo`**——
裸 shell 里 `CARGO_HOME` 会指到系统用户目录，缓存只读、编译起不来）：

```text
node dev.js test      # L1 单元（cargo test --bin solomni）
node dev.js fmt       # T0 格式（cargo fmt --all -- --check）
node dev.js clippy    # T0 静态检查（cargo clippy --all-targets --all-features --keep-going -- -D warnings）
```

纯**叙述型文档**改动用审查模式（不跑 cargo、单元、平台目标与 L4，只读仓库并写本次报告）：

```text
node run-tests.js --docs             # 文档审查 + 结构审查（本地纯文档任务）
node run-tests.js --audit=docs       # 只跑文档审查（CI docs job）
node run-tests.js --audit=structure  # 只跑结构审查（CI structure job）
```

`--docs` 同时跑 `docsAudit()`（文档自身：门户存在 / 链接、根文档路由）与 `structureAudit()`（代码↔文档结构：unit-map、提示词键表、systools、links.yaml、注释契约、依赖方向、测试目标 / 孤儿 / `#[ignore]` 等；不编译），
报告标 `mode: "docs"`。CI 把两者拆成独立的 `docs` / `structure` job，`--audit=docs|structure` 只跑其中一路（structure 报告 `mode: "structure"`）。
`docs/presentation/contracts.md` 是机器契约文档（ROUTES 表由 Rust 测试消费），改它仍要走全量；同时它是文档，文档审查也会查它的链接。
多个模式同时出现时 `--coverage` 优先于审查模式，`--docs` 优先于 `--audit`；`--skip-audit` 只作用于全量模式。纯文档任务不要为了凑流程跑全量。

迭代到一定段落、要按门禁判一次时用完整入口的**质量 job 模式**（T0 全套 + L1 单元 + 四个平台目标 + 前端冒烟 + 开发回路自检 + 环境引导自检，只省 L4）：

```text
node run-tests.js --skip-e2e
```

注意：`node dev.js test` 与 `node dev.js fmt`、`node dev.js clippy` **都预期全绿**——T0 九项里七项零容忍硬失败，没有容差基线（注释契约的存量快照只减不增）；交叉类型检查对已装 target 零容忍（未装 target 或本机缺交叉 C 工具链时 `env-skip`），供应链在工具缺失或取不到 advisory 数据时 `env-skip`。
用例默认**并行**跑；排查并发 / 隔离问题时用 `node run-tests.js --serial`。找测试盲区用 `node run-tests.js --coverage`
（见下文「覆盖率发现模式」，只报不拦）。

### 完整本地入口

```text
node run-tests.js
```

当前入口的实际顺序是：

1. `cargo build`（并打印安全模式 / 真机围栏模式说明）；
2. 运行 `solomni --doctor`，记录当前平台和围栏能力；
3. `T0 编译（--all-targets）`（硬失败）；
4. `T0 结构审查`（文档审查 + 结构审查；硬失败；逐项判定见 [levels.md](levels.md) 的 T0）；
5. `T0 格式（fmt --check）`（硬失败）；
6. `T0 静态检查（clippy）`（硬失败）；
7. `T0 编译告警`（硬失败）；
8. `T0 交叉类型检查（各已装 target）`（**开发机专属**：`cargo check --target … --all-targets` 兜平台专属 `#[cfg]`（含测试）的类型错误；target 未装或本机缺交叉 C 工具链 = `env-skip`；**CI 上记 `skip-platform` 直接跳过**，因为三平台矩阵各自原生 `--all-targets` 已覆盖）；
9. `T0 依赖重复（cargo tree）`（硬失败）；
10. `T0 供应链（audit/deny）`（工具缺失或取不到 advisory 数据 = env-skip）；
11. `L1 单元（--bin solomni）`（默认并行；`--serial` 改为串行，仅诊断用）；
12. 逐个运行四个测试目标 `cross-platform` / `windows` / `linux` / `macos`（同样默认并行；非本平台且整目标为空的记 `skip-platform`）；
13. 前端冒烟；
14. 开发回路自检（`tests/dev-loop.mjs`：`dev.js` 的子命令表与"环境没就绪就如实拒绝"）；
15. 依赖哈希工具自检（`tests/pins-tool.mjs`：`pins.js` 的台账解析 / 来源推导 / 原地重写；不碰网络）；
16. 变更分流自检（`tests/ci-changes.mjs --self-test`：`code_changed` / `docs_changed` 两个谓词、`include_str!/include_bytes!` 文档消费、失败兜底）；
17. 环境引导自检（`tests/env-bootstrap.mjs`：`env.js` 的下载 / 续传 / 结构判定 / sha256 闸，见下文；仅 Windows 真跑，时限 5 分钟）；
18. 存在编排器时运行 `L4 端到端`（编排器会**现场构建 indexer**：编译器版本进日志，构建失败即失败；`--skip-e2e` 时这一步整步不出现）；
19. `T0 项目外写`（快照项目外缓存根，新增 / 变化即硬失败；顺带清理系统临时目录里上次崩溃残留的 `solomni-*`）；
20. 写入 `target/test-report.json` 并打印 `TEST-REPORT-OK` 或 `TEST-REPORT-FAIL`；平台缺口账为空时再打印 `TEST-REPORT-ACCEPTED`。

`--docs` / `--audit=docs|structure` 是**独立模式**：只跑上面的第 4 步对应的审查（`--docs` 两者都跑），报告 `mode: "docs"` / `"structure"`，不打印 `TEST-REPORT-ACCEPTED`。

每一步都有墙钟上限（默认 15 分钟；L4 20 分钟，环境引导自检 5 分钟）：超时即硬失败，证据记进报告的 `timeouts`，不会把本地入口或
CI job 拖到外层超时。每步用时记在报告的 `ms`，超预算的标 `[slow]`（只报不拦）。

报告里的 `L1` / `L4` 是入口的步骤名：`L1` = T1 单元，`L4` = T5 端到端；四个测试目标按目标名逐步记录。

T0 与业务测试在同一次运行里出结果，但结论分开记：质量失败不能被业务测试通过抵消，反之亦然。

> L4 端到端在 `tests/cross-platform/e2e/root/` 下**现场生成**夹具：`orchestrator.js` 把产品根的
> `prompts/`、`systools/` 与三个真模块（`harvest` / `render` / `indexer`）拷进去再启动产品。那是**运行时副本、不是文档面**——
> 不要手改、不要当文档审计；要改就改产品根的那一份，副本每次重跑都会覆盖。
> 仓库有项目内工具链（`.tools/toolchains/active.json`）时，编排器把 `.tools/` 链进夹具根：产品按**产品根**找工具链，
> 夹具根没有就会回落宿主 PATH；宿主解释器装在用户目录（scoop / nvm）时容器围栏里跑不到。没有项目内工具链（如 CI 未装置备）则保持宿主回退。

### 环境引导自检（`tests/env-bootstrap.mjs`）

`env.js` 跑在门禁与产品**之前**（`start.bat` / `node env.js setup`）：它坏了等于用户装不上环境。这条自检用
**真实子进程**（`<临时根>/env.js setup --yes`）+ **本地应答源**把 Windows 的 MinGW 引导链逐条钉住；只写
`target/test-scratch/`、不碰外网、不装 Rust（给一个假 cargo 文件）。原样跑它：`node tests/env-bootstrap.mjs`。

| 场景 | 判据 |
| --- | --- |
| 整份下载 | 过 sha256 闸、解出 `dlltool.exe`、归档与来源标记都删掉 |
| 半截 + 同源标记 | 打向**记录下来的那个源**、带 `Range: bytes=<分片末尾>-`、续完装好 |
| 半截但没标记 | 从头下（不带着未知来源续传） |
| 结构不对（能下完但不是归档） | 退出 1、明说结构不对、删掉归档、不装 |
| sha256 不符 | 退出 1、报台账值 / 实际值 / 来源、**保留文件**；只有 `SOLOMNI_ACCEPT_ASSET_CHANGE=1` 放行 |
| 源不认 Range / 传到一半断开 | 退出 1、明说下载没完成、分片与来源标记都留着 |

非 Windows 上那条路（`dlltool`）不存在：先打一条 `[探针]` 再照常打印 `ENV-BOOTSTRAP-OK`——env-skip，不算通过。

### 依赖哈希检查（`pins.js`）

`pins.js` 是维护工具，不在产品运行期用：`update` 按上游校验源刷新 `links.yaml` / `dependencies.yaml` 的哈希，`check` 只读比对，`bump smbd <tag>` 把 smbd 的 URL 与哈希一起换。

- 普通 PR 的 `structure` 只做**离线**检查：键配对、64 位十六进制、无死数据；不联网。
- `pins-check` workflow 只在手动与每周一跑 `node pins.js check`，只读、不写文件、不开 PR；失败只说明当前 pin 与上游不一致（或上游取不到）。
- 它不判断“有没有新版本”：当前 pin 的 URL 仍自洽时 check 会通过；新版本靠发布通知 / 人工 `bump`。

### 真机入口（本地）

```text
node run-tests.js --fence-live
```

仅允许在一次性 runner、VM 或明确授权的环境使用：它会改本机状态（写目录 ACL、建容器 profile）并创建容器身份。
普通开发机上**不要**开；本地默认安全模式（见 [quality-isolation.md](quality-isolation.md)）。

**本地门禁的前提：仓库根没有被 Windows 写沙箱打上 Low 完整性标签。**
DSH 的 Windows 写沙箱后端（`@deepseek-ai/dsh-sandbox-windows-acl`）在授写权时给授权根设置
`SYSTEM_MANDATORY_LABEL_ACE`（Low、no-write-up、`(OI)(CI)` 可继承）。标签按**映像文件**生效：
Windows 的进程完整性 = min(令牌, 映像)。但标签只对**继承到它的对象**生效：根目录打标后新建的子项才继承，
已存在的子目录（例如 `target/`）可能没有标签，里面的测试二进制仍以 Medium 运行——所以"仓库根里的
cargo / rustc / 链接器 / 测试二进制 / solomni.exe 全以 Low 运行"并不总成立。判据要按**具体路径**查，
不要只看根。它不随会话结束自动撤销，回收要 `SeRelabelPrivilege`。

带标签时以下失败都是**假失败**，不是产品缺陷：

- 写 `%TEMP%`（中完整性）被拒：链接器报 `Cannot create temporary file in …\Temp\: Permission denied`，
  产品自检报 `建自检目录失败：拒绝访问`（`--doctor` 的 `fs=false`），真机探针只能 env-skip；
- 改目录 DACL 被拒（错误码 5）：容器围栏装不上；
- 读用户目录下的解释器被拒：doctor 报「没有 python」、L4 真工具场景报 python 不可达；
- 拉子进程 / 抓管道受会话沙箱限制（Node 管道 stdio 被 EPERM、`status` 为 null）：把 stdout 重定向到文件再读
  （`stdio: ["ignore", fd, "ignore"]`），不要用管道。这一条与 Low 标签是两件事。

**判据（唯一）**：对**根和实际要跑的可执行文件**分别跑 `icacls <路径> | findstr Mandatory`；任一处打出
`Mandatory Label\Low Mandatory Level` 就按下面处置。不要只看 `whoami /groups`：shell 可能显示 Medium，
决定进程完整性的是映像文件上的标签。

**处置，按优先级：**

1. 以「完全权限」启动会话：没有文件写授权就不会打标签，也不留残留（首选）；受限会话里可仅为单次执行申请放宽，批准范围仅限该次，且不替代真机围栏验收。
2. 提权还原级别（清残留）：`icacls "<根>" /setintegritylevel "(OI)(CI)Medium" /T /C`（需要 `SeRelabelPrivilege`，只能在管理员窗口做）；之后用同一条命令复查，不应再有 `Mandatory` 行。
3. 把二进制复制到未打标记的目录再跑（临时绕过）：`TEMP` 不要指进被标记的树，探针落在系统临时目录。
4. 换一台不受限的机器或交给 CI：跨平台与真机结论以 CI 为准。

随 DSH 分发的 `diagnose-windows-sandbox-acl` 只修 **ACL 侧**（补 `WRITE_DAC` / `WRITE_OWNER`、删显式 AppContainer 包 ACE），
不碰完整性标签；产品的快照 / 还原只带 `DACL_SECURITY_INFORMATION`（标签在 SACL）。标签残留按上面第 1/2/3 条处理。

> 带 Low 标签时的一切围栏 / L4 结论都不可信：doctor 报 `fs=false`、读写 DACL 都 Error 5、python 在工具进程里不可达。
> 清掉标签后同一条 `node run-tests.js --fence-live` 应全绿、doctor 报 `fs=true net=true tree=true`。
> 换普通或提权（`High`）的 shell 不解决——映像是 Low，进程照样是 Low。

### 虚拟机档真机用例（本地 opt-in）

虚拟机档真机用例默认跳过，设 `SOLOMNI_VM_LIVE=1` 才真跑（按需再给 `SOLOMNI_VM_QEMU` / `SOLOMNI_VM_GUEST`，虚拟根默认 `debian`）：

- `src/tests/vm.rs`：guest 起停、并发可写层；**取消与超时**用不依赖共享后端的 `vm_live_cancel_kills_the_guest_process_tree`（`NoneShare` + guest 内 `/tmp` 围栏；取消回执含「用户停止」且 15 秒内返回、无残留 `sleep`；`timeout_secs` 到点回执含 helper 的「工具进程超时」）。取消路径的 helper stderr 可能被 QGA 丢掉（时有时无），以无孤儿 + 快速返回证明处理器生效；
- `src/tests/builds.rs`：本机构建、缓存、失败与创建前补编（guest 内构建在 `SOLOMNI_VM_LIVE=1` 时真跑）；
- `src/capabilities/provision/guest.rs`：Linux 运行期编排；
- `src/capabilities/provision/service.rs`：guest-rootfs 属性真机用例（`guest_rootfs_pipeline_keeps_source_attrs_on_the_real_image`，用 `.vm/cache/rootfs.ext4`）；
- `tests/cross-platform/e2e/`：vm 会话场景。

未开启时用例打印行首 `[探针]`，入口按 `env-skip` 记进报告，不算通过。

**Linux 的 ext4 工具链用例按环境判据跳过**：`unshare -r` / `mke2fs` / `debugfs` 不可用（Linux CI 就是这样）时，
`packed_guest_image_uses_4k_blocks` 与 `guest_rootfs_restores_privilege_bits_and_ownership` 打印 `[探针]` 并跳过；
跳过 = `env-skip`，不是通过。补装 e2fsprogs 并允许非特权用户命名空间后才真跑。

### 覆盖率发现模式（手动）

```text
node run-tests.js --coverage
```

它**只用来找盲区，不做通过判据、不设阈值**（[TESTING.md](../../TESTING.md) 一：覆盖率高不代表行为契约被验证）。
工具缺失（没装 `cargo-llvm-cov` 或 `llvm-tools-preview`）记 env-skip 并打印安装命令，不算通过。
两个必须记住的局限：① 平台 `#[cfg]` 在别的平台根本不编译，覆盖率必须**逐平台**看；
② 产品二进制（守门进程、L4 端到端）由本入口单独 `cargo build`、**未插桩**，进程内覆盖率不包含它们。
报告落 `target/coverage/`；未覆盖的生产路径逐条决定补测或记 [tests/gaps.yaml](../../tests/gaps.yaml) 的
`testing.coverage-child-and-platform`。

### 突变测试工作流（手动）

范围只有两个：`core`（小而精，先跑）与 `capability`（单能力抽查）；**刻意不做 `all`**——全量收益低
（平台 `#[cfg]` 在别平台不编译、unviable/等价变异体多）、耗时长，单能力抽查已够用。
范围是 `tests/mutation-scope.json` 里的**文件级**清单（不写变异体名，避免行号漂移后腐烂）。

```text
（本地，需自己装 cargo-mutants）
MUTATION_SCOPE=core node tests/ci-mutation.mjs
MUTATION_SCOPE=capability MUTATION_CAPABILITY=repair node tests/ci-mutation.mjs
（CI 用 .github/workflows/mutants.yml，手动派发）
```

入口与门禁共用同一份环境解析（[env.js](../../env.js)）：借用系统 `cargo` 可以，但 `CARGO_HOME` 与临时目录仍钉在项目内。

**开发环境同步**：换机器 / 换人开发时跑 `node setup-dev.js`（`--check` 只读校验）。它统一用**项目内**工具链
（`platform/<os>/rustup`、`platform/<os>/cargo`），组件、交叉 target 与项目内工具都装那里——**不写系统 `~/.rustup`、`~/.cargo`**；条目清单见 [dev-tools.js](../../dev-tools.js)。

突变测试的测试命令固定「只跑 `--bin solomni`」并按串行跑（逐条变异单独复现，与门禁的默认并行无关）。`cargo-mutants` 按
「工具获取」策略在 CI 预编译装（见 [quality-isolation.md](quality-isolation.md)；一次性临时环境，不改本机）。已知**等价变异体**在仓库根的 `.cargo/mutants.toml`
的 `exclude_re` 里排除（每条都要写明为什么等价，当前 1 条）。

退出码语义（cargo-mutants）：`0` 全捕获、`2` 有未捕获、`3` 有超时、`4` 基线就挂。
只要不是 `0`，工作流变红并打印 `MUTATION-FOUND`（全捕获打印 `MUTATION-OK`）——**这是给人看的调查结果，
不参与 `TEST-REPORT-ACCEPTED`，也不写 `ci-report`**。

结果分两处，**各推各的**（`ci-report` 由 `test.yml` 独占，突变不碰它）：

- **`ci-mutation` 滚动分支**（小文本，可 `git show`，无凭据也能读）：
  `git fetch origin && git show origin/ci-mutation:capability-repair/missed.txt`；键是 `core` 或 `capability-<名字>`，
  **同一键每次覆盖、只留最近一次**，内容为 `report.json` / `meta.json` / `missed.txt` / `caught.txt` /
  `timeout.txt` / `unviable.txt` / `outcomes.json`；
- **Actions 产物 `mutation-report`**（完整现场）：`target/mutation-report.json`、`target/logs/mutation-*.log`、
  `mutants.out/`；分支只留最近一次，历史看这里。

两个范围都只作调查：全捕获 = `MUTATION-OK`，否则 `MUTATION-FOUND`；`capability` 范围按需派发。棘轮基线只在出现「已知容忍」的未捕获时再建。

### CI（GitHub Actions）：跨平台与真机的参考事实来源

跨平台验收工作流是 `.github/workflows/test.yml`，**两种触发：PR（目标 `main`，且已 Ready for review）与手动
`workflow_dispatch`**；`push` 不触发。Draft PR 的 `opened` / `synchronize` 仍会创建 run，但所有 job 跳过
（不耗 runner，也不发布 `ci-report`）；标 Ready（`ready_for_review`）才跑验收，之后的推送同样重跑。
PR 验收 PR head 提交（`SOURCE_SHA`），报告的 `sha` 因此等于 PR 头；手动派发验收 `github.sha`。
矩阵 `windows-latest / ubuntu-latest / macos-latest`（`fail-fast: false`，一个平台失败不影响另外两个出结论）。
来自 fork 的 PR 照跑测试，但不发布 `ci-report`（token 只读）。

平台相关的 job 是 `quality` 与 `e2e`；`changes` / `docs` / `structure` / `gate` / `publish-report` 是平台无关的 job：

| job | 跑什么 | 对应入口 |
| --- | --- | --- |
| `quality` | 平台门禁 + 供应链 + L1 单元 + 平台探针 + 前端冒烟 + 开发回路 / 变更分流 / 环境引导自检（**跳过 L4 与结构审查**） | `node run-tests.js --fence-live --skip-e2e --skip-audit` |
| `e2e` | L4 端到端（自己构建产品，与 `quality` 并行） | `node tests/ci-e2e.mjs`（CI 专用；本地整跑用 `node run-tests.js --fence-live`） |
| `changes` | 判定 `code_changed` / `docs_changed`；从 base 取分类器副本，自测 + `include_str!/include_bytes!` 文档消费自检 | `node tests/ci-changes.mjs`（副本） |
| `docs` | 有文档变更：文档审查（门户 / 链接 / 根文档路由） | `node run-tests.js --audit=docs` |
| `structure` | 始终跑：结构审查（代码↔文档结构；文档与真相源比对 + 纯文本检查） | `node run-tests.js --audit=structure` |
| `gate` | 汇总四个 job 的结果，规则集里唯一代表代码/文档验收的必填检查；不产报告 | 内联脚本读 `needs.*.result` |
| `publish-report` | 汇总三条 lane 的报告并发布 `ci-report` | `tests/ci-merge.mjs` + `tests/ci-publish.mjs` |

**三路分流**：`changes` 输出两个谓词：

- `code_changed`：至少一个非叙述型文档路径（`docs/presentation/contracts.md` 也算代码——ROUTES 表由 `src/tests/routes.rs` 的 `include_str!` 消费）；
- `docs_changed`：至少一个文档路径（`*.md`、`docs/**`、`LICENSE`；contracts.md 同时算文档，文档审查也查它的链接）。

| PR | `docs` 文档审查 | `structure` 结构审查 | `quality` / `e2e` 三平台 | `gate` |
| --- | --- | --- | --- | --- |
| 纯文档 | 跑 | 跑 | 跳过（闸门判为合法） | 跑 |
| 纯代码 | 跳过（闸门判为合法） | 跑 | 跑 | 跑 |
| 混合 | 跑 | 跑 | 跑 | 跑 |

**结构审查始终跑**：代码变更会破坏代码↔文档结构（新增 `src/**/*.rs` 没进 unit-map），文档变更同样会
（删 unit-map 一行、删提示词键表一个键、从 `SYSTOOL.md` 删表名）。所以它不是“纯文档检查”，而是两边都要对得上的门禁。
它平台无关，只跑一次，不放进三平台的 `quality`。

`quality` / `e2e` / `docs` 用 **job 级 `if` 跳过**，被跳过的 required check 计为 Success——但这条只对**非矩阵 job** 成立。
`docs` 名字固定，跳过就上报一条同名的 skipped；`quality` / `e2e` 是矩阵 job，而 `jobs.<job_id>.if` 在**矩阵展开之前**求值
（[GitHub 文档](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idif)），
被跳过时不会展开成 `（windows-latest）` 等名字，只报一条名字带字面量 `${{ matrix.os }}` 的 skipped。按平台名配置的
required context 因此永远停在 pending——纯文档 PR 会卡在「检查全绿/跳过，但不能合并」。所以平台 job 不进规则集，
改由固定名字的 `gate` 汇总。
**不要改成 workflow 级 `paths-ignore`**：workflow 不启动时 required context 根本不会上报，会永远停在 pending。

`gate`（检查名 `验收闸门`）在非 Draft 的 run 里始终运行，只读四个 job 的 `needs.*.result`，按 `changes` 的分类结果判「跳过是否合法」：
`code_changed=false` 时 `quality` / `e2e` 可以 skipped，`docs_changed=false` 时 `docs` 可以 skipped，`structure` 必须 success；
其余任何 skipped / failure / cancelled 都判失败。`changes` 自身不参与判定（它只是路由；失败时下游已按全量兜底），
所以分类器故障不会单独阻塞合并，但真跑挂了的 job 一定会被闸门拦下。`gate` 不写 `ci-report`，结论只在 PR 检查项里。

`changes` 先跑分类器自测；全量入口的「变更分流自检」步骤也会跑同一个 `--self-test`。分类器**从 base 取副本**（`git show base:tests/ci-changes.mjs`）再执行——PR 改自己的分类脚本不能把自己判成全文档；base 还没有该文件时两路都跑。取不到 diff、空 diff 或未知路径时同样落回全量。
新分类器写 `code_changed` / `docs_changed`；分类字段缺失（例如 base 的分类器还没有该键）时，下游条件按 `!= 'false'` 兜底为全量跑，不会出现启动期全跳过。

`publish-report` 把三条 lane 的产物（`structure-report`、`docs-report`、`test-report-<os>` 与 `e2e-report-<os>`）汇总：
平台的 quality / e2e 两份先用 `tests/ci-merge.mjs` 合并成该平台唯一的 `test-report.json`，再由 `tests/ci-publish.mjs` 发布。
**任一半缺报告都补一条 `fail` 步骤**——"没跑到"不能读成"通过"。
两份产物的上传根必须都是 `target/`（发布端把它们 `cp` 进 `target/` 再找 `target/e2e-report.json`）；e2e 的夹具日志先收进 `target/e2e-logs/` 再上传，
否则上传根会被抬到仓库根，报告落成 `target/target/e2e-report.json` 而被判成"没产出"。发布步骤为此留了一条布局检查：落到嵌套路径就让发布红，不静默丢 L4。
Draft 的 run 里依赖 job 都跳过，发布也跳过：不拿空产物发报告。

**报告统一到 `ci-report`**：该分支每次运行都会被重建为空分支再写入，只留最近一次运行。目录按 lane 分键：
`runs/structure/`、`runs/docs/`、`runs/<os>/`。文档审查 / 结构审查与三平台共用同一个分支。
纯代码 PR 没有 `runs/docs/`，纯文档 PR 没有平台目录；读的时候按存在的 key 比 `sha`。`gate` 不写报告，别在 `ci-report` 里找它。

> **规则集**：`main` 的必填检查是 `文档审查`、`结构审查`、`验收闸门`。
> **不要**把六个平台 job 或 `变更分流` 设为 required：平台 job 是矩阵 job，纯文档 PR 跳过时不会产出 `（windows-latest）` 这类名字（永远 pending）；
> `变更分流` 只是路由 job，卡住不应阻塞合并。纯文档 PR 的平台 job 是 skipped，由 `验收闸门` 判为合法；
> 纯代码 PR 的文档审查是 skipped，闸门同样只核对实际跑了的 job。本仓的规则集修改由用户在 GitHub 设置里执行。

`workflow_dispatch` 的 `clean` 输入（`true`）跳过 `actions/cache` 按干净机器跑；PR 触发没有这个输入，缓存照用。缓存只覆盖**项目内**路径
（`platform/ci/cargo`：借用 runner 自带 Rust 时的 `CARGO_HOME`，含 registry / git / 工具；以及 `target/debug`），
**不缓存 `target/` 根**（报告与日志必须来自本次运行）。缓存身份包含 `path`，改路径即新缓存项、旧项自动淘汰，首次冷启动重下一次。
CI 借 runner 自带 Rust（`CARGO_HOME` / `SOLOMNI_CARGO_HOME` 指项目内），不跑 `rustup` 安装——不写项目外。
供应链工具在进门前预编译装（工具获取策略见 [quality-isolation.md](quality-isolation.md)）；
安装用时经 `SOLOMNI_CI_PREINSTALL_MS` 带进门禁报告，在 `steps` 里以「CI 前置：供应链工具安装」出现——安装不在 `run-tests.js` 内，只能这样进报告。

**为什么必须有 CI**——下面这些结论本地拿不到：

| 场景 | 本地为什么不够 | CI 提供什么 |
| --- | --- | --- |
| 平台专属代码（`kernel/detail/confine/` 各平台文件、`tests/<平台>/`） | 平台目标的 `main.rs` 首行是 `#![cfg(target_os = …)]`：非本平台的目标整目标为空，代码根本不编译 | 三平台各编译并各跑一次 |
| 真机围栏（ACL / 容器 profile / Landlock / seatbelt） | 本地默认安全模式会跳过会改本机状态的探针 | 一次性 runner 上真跑，并验撤权与 profile 回收 |
| HTTPS/TLS 出站链路 | 受限环境可能取不到系统 TLS 凭证（判据见 [levels.md](levels.md) 的 T4），本地只能 env-skip | 干净 runner 上真连公网端点 |
| T0 九项（clippy 只编译当前平台的 `#[cfg]`；交叉类型检查在本机无目标或交叉 C 工具链时 env-skip；依赖图随平台变；供应链要联网装工具） | 本机只能代表本平台（判的是**平台差异**，不是「改了门禁就要派发」） | 三平台各跑同一套平台相关门禁（结构审查由独立 `structure` job 跑一次）：硬失败项零容忍，环境项记 `env-skip` |
| 三种语言的模块（python / node / C++）在真进程里跑 | 本机只代表本平台的解释器与编译器 | 三平台各跑一次真工具链路，indexer 现场编译 |
| 发布前验收 | 本地通过 ≠ 三平台通过 | 三平台报告 + 三平台 `TEST-REPORT-ACCEPTED` |

文档审查与结构审查本身是本地可复现的确定性检查；CI 里跑它们是为了 required check 与合并闸门，不是补平台结论。

**什么时候手动派发**：Draft 阶段不自动跑 CI，需要他平台结论时手动派发；PR 标 Ready 后自动跑，不必再派发。
手动派发还用于「没有 PR 的分支」与「干净机器复验」。
判据只有一条——这次改动的验收结论**是否依赖本机之外**（其它平台能不能编译通过、真机围栏与解释器链路、TLS 出站）：

- **依赖** → Draft 期手动派发一次（或直接 Ready 让 PR 跑），并按下面的读法比对 `sha`；
- **不依赖** → 本地入口就是验收结论；PR 标 Ready 时会照跑，但不把 CI 的 `env-skip` 当通过。
- **纯叙述型文档** → 不依赖本机之外：本地 `node run-tests.js --docs`（文档审查 + 结构审查）通过即可；PR 标 Ready 时走 `changes` + `docs` + `structure` + `gate`，不产生三平台验收。
- 用户明确要求派发时例外。

**触发**：PR 标 Ready（或对 Ready 的 PR 推送）即自动跑；Draft 不跑。需要单独验证某个分支或干净机器时手动派发，
用 `gh` 发起，`--ref` 指向要验证的分支（`main` 禁止直接 push，通常是 `development`）：

> **Draft 的 run 只证明工作流文件被接受**（YAML 解析通过）：所有 job 跳过、不产报告、不发布 `ci-report`，
> 不能当任何验收结论。要验证工作流本身（产物上传/下载、合并与发布链路），必须手动派发或等 Ready 后跑一次。

```text
gh workflow run test.yml --ref <分支>
```

按干净机器跑（跳过缓存）时加 `-f clean=true`。Ready 的 PR 不必另派发；触发后按下面的读法比对 `sha`。

**报告怎么读（硬规矩）**：只用 `git` 或 git CLI 拉滚动分支，**禁止轮询网页**；时机无法确认时委托用户拉取（见 `AGENTS.md`）。
代码 / 文档 / 结构验收都读 `ci-report`：

```text
git fetch origin
# 结构审查（始终有）
git show origin/ci-report:runs/structure/meta.json
git show origin/ci-report:runs/structure/test-report.json
# 文档审查（有文档变更时才有）
git show origin/ci-report:runs/docs/meta.json
git show origin/ci-report:runs/docs/test-report.json
# 三平台（有代码变更时才有）
git show origin/ci-report:runs/windows/meta.json        # run / sha / mode / failed / qualityFailed
git show origin/ci-report:runs/windows/test-report.json  # 与本地 target/test-report.json 同构
git show origin/ci-report:runs/windows/logs/<某一步>.log  # 失败证据原文
```

`runs/<key>/` 的 key 取 `structure` / `docs` / `windows` / `linux` / `macos`；纯文档 PR 只有前两个，纯代码 PR 只有 `structure` + 三个平台。判定顺序：

1. **先比 `sha`**：`meta.json` 的 `sha` 必须等于要验收的那个提交；不等 = 这次 CI 还没覆盖它，别拿旧结论当新证据。
2. 再看 `failed` 与 `qualityFailed`（都为假才算通过）。
3. 再读 `test-report.json` 的 `steps` 与 `envSkips`：**CI 上的 env-skip 同样不算通过**，它只说明那条围栏没被验收。
4. 失败时从 `logs/` 取断言原文，不在摘要里找感觉。

> `envSkips` 的来源是探针级 `[探针]` 标记：`run-tests.js` 只认行首（`skipsIn`），`tests/ci-e2e.mjs` 按包含匹配。
> 诊断输出一律用 `[诊断]`，别用 `[探针]`，否则会被算成"跳过"。
>
> 判「环境降级」只看**被测试进程**的令牌与行为：`whoami /groups` 里找包 SID 组在现代 Windows 上是**假阴性**
> （包 SID 在令牌的 `TokenAppContainerSid` 字段里，不在组列表里），按它判会把生效的容器记成降级。
> 容器是否生效用**行为对照**（授权落点写得进、父目录按名可用、父目录内容不可见），见 [levels.md](levels.md) 的 T4。

**等多久再拉（推荐节奏）**：触发之后（PR 标 Ready 或手动派发）**先等 5 分钟**再拉 `ci-report`；若本次跑了的 lane 的 `meta.json` 的 `sha` 还对不上
（这次 run 没结束），**每次再等 2 分钟**重拉一次，直到对应 key 的 `sha` 都对得上（纯文档看 `runs/structure` + `runs/docs`；有代码看 `runs/structure` + 三平台），或确认 run 已失败/取消。
等的是 git 拉取，不是网页轮询——`AGENTS.md` 禁止查网页；时间上拿不准（比如 runner 排队很久）就委托用户拉取。

**发布通道**（由 `tests/ci-publish.mjs` 在 CI 里自助发布，成败都发）：

- Actions 注释：失败逐条 `::error::`（带断言原文），通过一条 `::notice::` 概览——匿名可读，不需要凭据；
- `ci-report` 滚动分支（三路共用）：同一路径每次覆盖，**只留最近一次**；结构审查在 `runs/structure/`，文档审查在 `runs/docs/`，三平台在 `runs/<os>/`（要历史看 Actions 产物 `test-report-<os>`）；
- **Ready 的 PR 与手动派发都发布** `ci-report`（`push` 不触发；Draft 的 run 全部跳过，不发布）；PR 报告的 `sha` 是 PR head（fork PR 不发布，见上）；
- 报告发布失败（例如 token 权限不对）只是 `::warning::`，**不影响**测试本身的成败——所以"没读到报告"不等于"测试没过"。

**本地与 CI 的关系**：本地入口是快速反馈；CI 在 Ready 的 PR 上自动跑（Draft 期按需手动派发），给出**跨平台与真机**的最终判据。
本次改动若结论依赖本机之外，则本地 + CI 两边都跑通、且 CI 的 `sha` 对得上才算验收完成；
否则本地入口通过即验收完成（PR 的 CI 结果照读，`env-skip` 不算通过；判据见 [gaps-acceptance.md](gaps-acceptance.md)）。

### 当前报告状态

当前实现实际会输出：

- `pass`：步骤完成；
- `fail`：断言或命令失败（硬失败，含编译、交叉类型检查、供应链、项目外写与业务用例）；
- `quality-fail`：结构审查 / 格式 / clippy / 编译告警 / 依赖重复不过；
- `env-skip`：工具缺失等环境性跳过（步骤级），与 `envSkips`（探针级的 `[探针]` 行）并列；
- `skip-platform`：当前平台不适用的空平台目标；
- `gap`：入口没有找到应运行的部分。

本地 `target/test-report.json` 的字段：`mode`（`full` / `docs` / `structure`；`--docs` 是文档审查 + 结构审查，`--audit=docs` / `--audit=structure` 只跑其中一路）、`platform` / `arch` / `osKey` / `profile` / `fenceLive` / `doctor`（本次运行的环境事实）、
`steps`（逐步骤状态、用时 `ms` 与超预算标记 `slow`——"哪一步慢"要看它，不看感觉）、`envSkips`（探针级跳过）、
`timeouts`（被墙钟超时终止的命令）、`quality`（`failed` / `steps`）、`gaps`（平台缺口账）、
`globalGaps`（`tests/gaps.yaml` 的长期目标）、`failed`（状态为 `fail` 的步骤数；`quality-fail` 另由 `quality.failed` 计）。
CI 合并后的平台报告由 `tests/ci-merge.mjs` 生成：`mode` 固定为 `full`，保留上述环境字段与 `steps` / `envSkips` / `quality` / `globalGaps` / `gaps` / `failed`，
不保留 `timeouts` 与 `steps[].slow`。
`mode: "docs"` / `mode: "structure"` 的报告不走 `ci-merge.mjs`（没有 e2e 半份），由 `tests/ci-publish.mjs` 直接发布到 `ci-report` 的 `runs/docs/` / `runs/structure/`；
它们的 `gaps` / `globalGaps` 为空，`steps` 只有 `T0 文档审查` / `T0 结构审查`（本地 `--docs` 是 `T0 文档门禁`），不代表平台缺口验收。
`runs/<key>/meta.json` 的 key 是 `structure` / `docs` / `windows` / `linux` / `macos`，也带 `mode`（`full` / `docs` / `structure` / `none`），读报告时先比 `sha`，再认 `mode`。
报告没有 `test-fail` / `blocked` 这两个状态：失败只有 `fail`，缺口只有 `gap`。

## 二、成功标记

固定标记只增不删，改动含义必须同步更新测试设计：

- `FRONTEND-SMOKE-OK`：前端冒烟完成；
- `DEV-LOOP-OK`：开发回路自检完成（`dev.js` 的子命令表与拒绝出口）；
- `ENV-BOOTSTRAP-OK`：环境引导自检完成（`env.js` 的下载 / 续传 / 结构判定 / sha256 闸；非 Windows 会先打一条 `[探针]`）；
- `E2E-OK`：端到端场景完成；
- `TEST-REPORT-OK`：当前入口的运行步骤没有失败；`--docs` / `--audit` 模式只代表对应审查（文档审查 / 结构审查）通过，不代表代码、三平台或 L4 已验收；
- `TEST-REPORT-FAIL`：当前入口有硬失败步骤**或** T0 任一项不过（同时以非零退出码暴露）；
- `TEST-REPORT-ACCEPTED`：当前平台缺口账为空；
- `MUTATION-OK` / `MUTATION-FOUND`：突变测试工作流全捕获 / 有未捕获或超时——**只用于调查，不参与验收**。

固定标记不能替代质量门禁，也不能覆盖 `env-skip`、`gap` 或 `quality-fail`。测试入口必须以非零退出码暴露失败。

