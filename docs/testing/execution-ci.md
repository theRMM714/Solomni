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
4. `T0 结构审查`（硬失败；逐项判定见 [levels.md](levels.md) 的 T0）；
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
15. 环境引导自检（`tests/env-bootstrap.mjs`：`env.js` 的下载 / 续传 / 结构判定 / sha256 闸，见下文；仅 Windows 真跑，时限 5 分钟）；
16. 存在编排器时运行 `L4 端到端`（编排器会**现场构建 indexer**：编译器版本进日志，构建失败即失败；`--skip-e2e` 时这一步整步不出现）；
17. `T0 项目外写`（快照项目外缓存根，新增 / 变化即硬失败；顺带清理系统临时目录里上次崩溃残留的 `solomni-*`）；
18. 写入 `target/test-report.json` 并打印 `TEST-REPORT-OK` 或 `TEST-REPORT-FAIL`；平台缺口账为空时再打印 `TEST-REPORT-ACCEPTED`。

每一步都有墙钟上限（默认 15 分钟；L4 20 分钟，环境引导自检 5 分钟）：超时即硬失败，证据记进报告的 `timeouts`，不会把本地入口或
CI job 拖到外层超时。每步用时记在报告的 `ms`，超预算的标 `[slow]`（只报不拦）。

报告里的 `L1` / `L4` 是入口的步骤名：`L1` = T1 单元，`L4` = T5 端到端；四个测试目标按目标名逐步记录。

T0 与业务测试在同一次运行里出结果，但结论分开记：质量失败不能被业务测试通过抵消，反之亦然。

> L4 端到端在 `tests/cross-platform/e2e/root/` 下**现场生成**夹具：`orchestrator.js` 把产品根的
> `prompts/`、`systools/` 与三个真模块（`harvest` / `render` / `indexer`）拷进去再启动产品。那是**运行时副本、不是文档面**——
> 不要手改、不要当文档审计；要改就改产品根的那一份，副本每次重跑都会覆盖。

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

### 真机入口（本地）

```text
node run-tests.js --fence-live
```

仅允许在一次性 runner、VM 或明确授权的环境使用：它会改本机状态（写目录 ACL、建容器 profile）并创建容器身份。
普通开发机上**不要**开；本地默认安全模式（见 [quality-isolation.md](quality-isolation.md)）。

**本地门禁的前提：仓库根没有被 Windows 写沙箱打上 Low 完整性标签。**
DSH 的 Windows 写沙箱后端（`@deepseek-ai/dsh-sandbox-windows-acl`）在授写权时给授权根设置
`SYSTEM_MANDATORY_LABEL_ACE`（Low、no-write-up、`(OI)(CI)` 可继承）。标签按**映像文件**生效：
Windows 的进程完整性 = min(令牌, 映像)，所以仓库根里的 `cargo` / `rustc` / 链接器 / 测试二进制 / `solomni.exe`
全以 Low 运行；它不随会话结束自动撤销，回收要 `SeRelabelPrivilege`。

带标签时以下失败都是**假失败**，不是产品缺陷：

- 写 `%TEMP%`（中完整性）被拒：链接器报 `Cannot create temporary file in …\Temp\: Permission denied`，
  产品自检报 `建自检目录失败：拒绝访问`（`--doctor` 的 `fs=false`），真机探针只能 env-skip；
- 改目录 DACL 被拒（错误码 5）：容器围栏装不上；
- 读用户目录下的解释器被拒：doctor 报「没有 python」、L4 真工具场景报 python 不可达；
- 拉子进程 / 抓管道受会话沙箱限制（Node 管道 stdio 被 EPERM、`status` 为 null）：把 stdout 重定向到文件再读
  （`stdio: ["ignore", fd, "ignore"]`），不要用管道。这一条与 Low 标签是两件事。

**判据（唯一）**：`icacls "<仓库根>" | findstr Mandatory` 打出 `Mandatory Label\Low Mandatory Level`（子项显示 `(I)` 继承）。
不要只看 `whoami /groups`：shell 可能显示 Medium，决定进程完整性的是映像文件上的标签。

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

- `src/tests/vm.rs`：guest 起停、并发可写层；
- `src/tests/diagnostics.rs`：guest 内构建；
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

### CI（GitHub Actions）：跨平台与真机的唯一事实来源

跨平台验收工作流是 `.github/workflows/test.yml`，**两种触发：PR（目标 `main`，且已 Ready for review）与手动
`workflow_dispatch`**；`push` 不触发。Draft PR 的 `opened` / `synchronize` 仍会创建 run，但所有 job 跳过
（不耗 runner，也不发布 `ci-report`）；标 Ready（`ready_for_review`）才跑验收，之后的推送同样重跑。
PR 验收 PR head 提交（`SOURCE_SHA`），报告的 `sha` 因此等于 PR 头；手动派发验收 `github.sha`。
矩阵 `windows-latest / ubuntu-latest / macos-latest`（`fail-fast: false`，一个平台失败不影响另外两个出结论）。
来自 fork 的 PR 照跑测试，但不发布 `ci-report`（token 只读）。

**契约：每个平台恰好两个 job**——`quality` 与 `e2e`；这是刻意的上限，**不得再拆**：

| job | 跑什么 | 对应入口 |
| --- | --- | --- |
| `quality` | T0 质量门禁 + 供应链 + L1 单元 + 平台探针 + 前端冒烟 + 开发回路 / 环境引导自检（**跳过 L4**） | `node run-tests.js --fence-live --skip-e2e` |
| `e2e` | L4 端到端（自己构建产品，与 `quality` 并行） | `node tests/ci-e2e.mjs`（CI 专用；本地整跑用 `node run-tests.js --fence-live`） |

`publish-report` 把两个 job 的产物（`test-report-<os>` 与 `e2e-report-<os>`）用 `tests/ci-merge.mjs` 合并成该平台
唯一的 `test-report.json`，再交给 `tests/ci-publish.mjs` 发布。**任一半缺报告都补一条 `fail` 步骤**——"没跑到"不能读成"通过"。
Draft 的 run 里两个依赖 job 都跳过，`publish-report` 也跳过：不拿空产物发报告。

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
| T0 九项（clippy 只编译当前平台的 `#[cfg]`；交叉类型检查在本机无目标或交叉 C 工具链时 env-skip；依赖图随平台变；供应链要联网装工具） | 本机只能代表本平台（判的是**平台差异**，不是「改了门禁就要派发」） | 三平台各跑同一套门禁：硬失败项零容忍，环境项记 `env-skip` |
| 三种语言的模块（python / node / C++）在真进程里跑 | 本机只代表本平台的解释器与编译器 | 三平台各跑一次真工具链路，indexer 现场编译 |
| 发布前验收 | 本地通过 ≠ 三平台通过 | 三平台报告 + 三平台 `TEST-REPORT-ACCEPTED` |

**什么时候手动派发**：Draft 阶段不自动跑 CI，需要他平台结论时手动派发；PR 标 Ready 后自动跑，不必再派发。
手动派发还用于「没有 PR 的分支」与「干净机器复验」。
判据只有一条——这次改动的验收结论**是否依赖本机之外**（其它平台能不能编译通过、真机围栏与解释器链路、TLS 出站）：

- **依赖** → Draft 期手动派发一次（或直接 Ready 让 PR 跑），并按下面的读法比对 `sha`；
- **不依赖** → 本地入口就是验收结论；PR 标 Ready 时会照跑，但不把 CI 的 `env-skip` 当通过。
- 用户明确要求派发时例外。

**触发**：PR 标 Ready（或对 Ready 的 PR 推送）即自动跑；Draft 不跑。需要单独验证某个分支或干净机器时手动派发，
用 `gh` 发起，`--ref` 指向要验证的分支（`main` 禁止直接 push，通常是 `development`）：

```text
gh workflow run test.yml --ref <分支>
```

按干净机器跑（跳过缓存）时加 `-f clean=true`。Ready 的 PR 不必另派发；触发后按下面的读法比对 `sha`。

**报告怎么读（硬规矩）**：只用 `git` 或 git CLI 拉 `ci-report` 分支，**禁止轮询网页**；时机无法确认时委托用户拉取（见 `AGENTS.md`）。

```text
git fetch origin
git show origin/ci-report:runs/windows/meta.json        # run / sha / failed / qualityFailed
git show origin/ci-report:runs/windows/test-report.json  # 与本地 target/test-report.json 同构
git show origin/ci-report:runs/windows/logs/<某一步>.log  # 失败证据原文
```

`runs/<os>/` 的 `os` 取 `windows` / `linux` / `macos`。判定顺序：

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

**等多久再拉（推荐节奏）**：触发之后（PR 标 Ready 或手动派发）**先等 5 分钟**再拉 `ci-report`；若某个平台的 `meta.json` 的 `sha` 还对不上
（这次 run 没结束），**每次再等 2 分钟**重拉一次，直到三平台的 `sha` 都对得上，或确认 run 已失败/取消。
等的是 git 拉取，不是网页轮询——`AGENTS.md` 禁止查网页；时间上拿不准（比如 runner 排队很久）就委托用户拉取。

**发布通道**（由 `tests/ci-publish.mjs` 在 CI 里自助发布，成败都发）：

- Actions 注释：失败逐条 `::error::`（带断言原文），通过一条 `::notice::` 概览——匿名可读，不需要凭据；
- `ci-report` 滚动分支：同一路径每次覆盖，**只留最近一次**（要历史看 Actions 产物 `test-report-<os>`）；
- **PR（Ready）与手动派发的 run 都发布** `ci-report`（`push` 不触发；Draft 的 run 全部跳过，不发布）；PR 报告的 `sha` 是 PR head（fork PR 不发布，见上）；
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

本地 `target/test-report.json` 的字段：`platform` / `arch` / `osKey` / `profile` / `fenceLive` / `doctor`（本次运行的环境事实）、
`steps`（逐步骤状态、用时 `ms` 与超预算标记 `slow`——"哪一步慢"要看它，不看感觉）、`envSkips`（探针级跳过）、
`timeouts`（被墙钟超时终止的命令）、`quality`（`failed` / `steps`）、`gaps`（平台缺口账）、
`globalGaps`（`tests/gaps.yaml` 的长期目标）、`failed`（状态为 `fail` 的步骤数；`quality-fail` 另由 `quality.failed` 计）。
CI 合并后的报告由 `tests/ci-merge.mjs` 生成：保留上述环境字段与 `steps` / `envSkips` / `quality` / `globalGaps` / `gaps` / `failed`，
不保留 `timeouts` 与 `steps[].slow`。
报告没有 `test-fail` / `blocked` 这两个状态：失败只有 `fail`，缺口只有 `gap`。

## 二、成功标记

固定标记只增不删，改动含义必须同步更新测试设计：

- `FRONTEND-SMOKE-OK`：前端冒烟完成；
- `DEV-LOOP-OK`：开发回路自检完成（`dev.js` 的子命令表与拒绝出口）；
- `ENV-BOOTSTRAP-OK`：环境引导自检完成（`env.js` 的下载 / 续传 / 结构判定 / sha256 闸；非 Windows 会先打一条 `[探针]`）；
- `E2E-OK`：端到端场景完成；
- `TEST-REPORT-OK`：当前入口的运行步骤没有失败；
- `TEST-REPORT-FAIL`：当前入口有硬失败步骤**或** T0 任一项不过（同时以非零退出码暴露）；
- `TEST-REPORT-ACCEPTED`：当前平台缺口账为空；
- `MUTATION-OK` / `MUTATION-FOUND`：突变测试工作流全捕获 / 有未捕获或超时——**只用于调查，不参与验收**。

固定标记不能替代质量门禁，也不能覆盖 `env-skip`、`gap` 或 `quality-fail`。测试入口必须以非零退出码暴露失败。

