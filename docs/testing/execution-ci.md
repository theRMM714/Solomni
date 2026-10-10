# 执行入口、报告与 CI

> 本文是**测试执行入口、报告状态、成功标记与 CI 流程的唯一权威**。
> 质量门禁见 [quality-isolation.md](quality-isolation.md)，验收清单见 [gaps-acceptance.md](gaps-acceptance.md)。

## 一、执行入口与报告

### 快速开发检查

```text
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

快速检查用于本地反馈，不替代完整入口。

注意：`cargo test` 与 `cargo fmt --check`、`cargo clippy … -D warnings` **都预期全绿**——T0 八项全是零容忍硬失败，没有存量基线。
用例默认**串行**跑（套件含真实线程时序用例，并行仍会偶发，见 [tests/gaps.yaml](../../tests/gaps.yaml) 的 `testing.parallel-flake`）；排查并发 / 隔离问题时用 `node run-tests.js --parallel`。找测试盲区用 `node run-tests.js --coverage`
（见下文「覆盖率发现模式」，只报不拦）。

### 完整本地入口

```text
node run-tests.js
```

当前入口的实际顺序是：

1. `cargo build`（并打印安全模式 / 真机围栏模式说明）；
2. 运行 `solomni --doctor`，记录当前平台和围栏能力；
3. `T0 编译（--all-targets）`（硬失败）；
4. `T0 结构审查`（硬失败：目标登记 / 孤儿测试文件 / 缺口账格式 / 文档链接完整性 / 门禁解析器自测 / `#[ignore]` 禁令）；
5. `T0 格式（fmt --check）`（硬失败）；
6. `T0 静态检查（clippy）`（硬失败）；
7. `T0 编译告警`（硬失败）；
8. `T0 依赖重复（cargo tree）`（硬失败）；
9. `T0 供应链（audit/deny）`（工具缺失或取不到 advisory 数据 = env-skip）；
10. `L1 单元（--bin solomni）`（默认串行；`--parallel` 改为并发，仅诊断用）；
11. 逐个运行 `cargo test --test cross-platform/windows/linux/macos`（同样默认串行）；
12. 前端冒烟；
13. 存在编排器时运行 L4 端到端（编排器会**现场构建 indexer**：编译器版本进日志，构建失败即失败）；
14. 写入 `target/test-report.json` 并打印 `TEST-REPORT-OK` 或 `TEST-REPORT-FAIL`。

每一步都有墙钟上限（默认 15 分钟，L4 20 分钟）：超时即硬失败，证据记进报告的 `timeouts`，不会把本地入口或
CI job 拖到外层超时。每步用时记在报告的 `ms`，超预算的标 `[slow]`（只报不拦）。

T0 与业务测试在同一次运行里出结果，但结论分开记：质量失败不能被业务测试通过抵消，反之亦然。

> L4 端到端在 `tests/cross-platform/e2e/root/` 下**现场生成**夹具：`orchestrator.js` 把产品根的
> `prompts/`、`modules/`、`systools/` 拷进去再启动产品。那是**运行时副本、不是文档面**——
> 不要手改、不要当文档审计；要改就改产品根的那一份，副本每次重跑都会覆盖。

### 真机入口（本地）

```text
node run-tests.js --fence-live
```

仅允许在一次性 runner、VM 或明确授权的环境使用：它会改本机状态（写目录 ACL、建容器 profile）并创建容器身份。
普通开发机上**不要**开；本地默认安全模式（见 [quality-isolation.md](quality-isolation.md)）。

**本地这条最后一行有个前提：门禁要在「工作区没有被打上低完整性标签」的环境里跑。**
DSH 的 Windows 写沙箱后端（`@deepseek-ai/dsh-sandbox-windows-acl`）在**给每个授权根授写权**的同一次
`SetNamedSecurityInfoW` 调用里，顺手给那个目录打上 **Low 完整性标签**（`SYSTEM_MANDATORY_LABEL_ACE`、
no-write-up、`(OI)(CI)` 可继承）。它有三个要命处：

- **跟着会话留下来**：`dispose()` 不撤销它——会话结束、甚至会话之后切成「完全权限」，标签都还在；
- **按「映像文件」生效**：Windows 的进程完整性 = min(令牌, 映像)，所以工作区里的一切二进制
  （`cargo`、`rustc`、链接器、测试二进制、`solomni.exe`）从被打标签那一刻起**全以 Low 运行**；
- **降级容易、回收难**：把对象降成 Low 不需要特权，撤/抬回去要 `SeRelabelPrivilege`（只有提权令牌有）。

后果是一串「指向错误方向」的假失败：

- 写 `%TEMP%`（中完整性）被拒 → 链接器报 `Cannot create temporary file in …\Temp\: Permission denied`、
  产品自检报 `建自检目录失败：拒绝访问`（`--doctor` 因此 `fs=false`）、真机探针只能 env-skip；
- 改目录 DACL 被拒（错误码 5）→ 容器围栏装不上；
- **读**用户目录下的解释器被拒 → doctor 报「没有 python」、L4 真工具场景报 python 不可达（看着像产品缺陷）；
- 拉子进程/抓管道另行受会话沙箱限制（Node 管道 stdio 被拒 EPERM、`status` 为 null，被判成「本机留下了没人管的痕迹」）——
  这一条与 Low 标签是**两件事**，但同样要求把 stdout 重定向到文件再读（`stdio: ["ignore", fd, "ignore"]`），别用管道。

**判据（唯一）**：`icacls "<工作区根>" | findstr Mandatory` 打出 `Mandatory Label\Low Mandatory Level`
就是它（子项显示 `(I)` 继承）。**不要只看 `whoami /groups`**：shell 自己可能显示 Medium，真正决定进程
完整性的是**映像文件**上的标签——这就是「要查对进程」的意思。

**处置，按优先级：**

1. **以「完全权限」启动会话**：没有文件写授权就不会打标签（首选，也是唯一不产生残留的做法）；
   只在受限会话里跑时，可为**单次执行**申请放宽到不受限——批准范围仅限该次，且不替代真机围栏验收。
2. **提权把级别设回去**（清残留）：`icacls "<根>" /setintegritylevel "(OI)(CI)Medium" /T /C`
   （只能在管理员窗口做，因为需要 `SeRelabelPrivilege`）；清完用同一条 `icacls` 复查，不应再有 `Mandatory` 行。
3. **把二进制复制到未打标记的目录再跑**（临时绕过）：注意别把 `TEMP` 指进被标记的树，探针要落在系统临时目录。
4. **换一台不受限的机器 / 交给 CI**：跨平台与真机的结论一律以 CI 为准。
另有一个**名字相近、用途不同**的官方工具：随 DSH 分发的自愈技能 `diagnose-windows-sandbox-acl`。
它只覆盖 **ACL 侧**——给缺有效 `WRITE_DAC` / `WRITE_OWNER` 的链上目录补当前用户全权、并删掉显式的
AppContainer 包 ACE（`S-1-15-2-*`）；**它不碰完整性标签**（脚本里只把 Low 标签读出来记一笔事实）。
两者不要混用：标签残留走上面第 1/2/3 条；它自己的场景是「DSH 授权失败 → 沙箱 fail-closed」，
且要在**没有产品会话运行**时用——它删掉的是产品**活着的**授权（容器会中途失去访问、那次会话直接失败；
产品收尾会把根内 DACL 按快照整体还原，但救不了已经失败的那一次运行）。两条边界互不相干：
产品的快照/还原只带 `DACL_SECURITY_INFORMATION`（标签在 SACL），所以它既不碰标签、也修不了标签。

> **工作区带 Low 标签时的一切围栏 / L4 结论都不可信**——实测（同一台机器、同一份代码）：标签在时
> doctor 报 `fs=false`、读写 DACL 都 Error 5、python 在工具进程里不可达、围栏回收因 Node 管道 stdio 被拒
> 被判成「本机留下了没人管的痕迹」；**把它清掉**之后同一条 `node run-tests.js --fence-live` 立刻
> `391 passed / 0 failed`、doctor 报 `fs=true net=true tree=true`、真工具链全跑通，`--fence-live` 的真机探针
> （容器往返、对象 ACE 往返、授权/撤销/台账/孤儿清扫）也在本机真跑通过。所以「卷不支持 ACL」「python 装得不对」
> 这类结论都是误判。**注意：换普通或提权（`High`）的 shell 本身不解决**——映像是 Low 的，进程照样是 Low；
> 要么不打标签，要么把标签清掉，要么把二进制搬出这棵树。

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

测试命令固定「只跑 `--bin solomni` + `--test-threads=1`」：**串行是刻意的**，并行会偶发
（见 [tests/gaps.yaml](../../tests/gaps.yaml) 的 `testing.parallel-flake`）。`cargo-mutants` 按
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

首轮 **core** 已分诊：9 个真缺口补测、1 个等价变异体在 `.cargo/mutants.toml` 排除；复跑
`45 caught / 3 unviable / 0 missed / 0 timeout` → `MUTATION-OK`。各 `capability` 可按需派发，首轮尚未逐个抽查；
棘轮基线暂不建（core 已归零，等出现「已知容忍」的未捕获时再加）。

### CI（GitHub Actions）：跨平台与真机的唯一事实来源

跨平台验收工作流是 `.github/workflows/test.yml`，**只有手动触发**（`workflow_dispatch`）：`main` 禁止直接 push（只走 PR），
日常提交不自动跑 CI。矩阵 `windows-latest / ubuntu-latest / macos-latest`（`fail-fast: false`，
一个平台失败不影响另外两个出结论）。

**契约：每个平台恰好两个 job**——`quality` 与 `e2e`；这是刻意的上限，**不得再拆**：

| job | 跑什么 | 对应入口 |
| --- | --- | --- |
| `quality` | T0 质量门禁 + 供应链 + L1 单元 + 平台探针 + 前端冒烟（**跳过 L4**） | `node run-tests.js --fence-live --skip-e2e` |
| `e2e` | L4 端到端（自己构建产品，与 `quality` 并行） | `node tests/ci-e2e.mjs`（CI 专用；本地整跑用 `node run-tests.js --fence-live`） |

`publish-report` 把两个 job 的产物（`test-report-<os>` 与 `e2e-report-<os>`）用 `tests/ci-merge.mjs` 合并成该平台
唯一的 `test-report.json`，再交给 `tests/ci-publish.mjs` 发布。**任一半缺报告都补一条 `fail` 步骤**——"没跑到"不能读成"通过"。

`workflow_dispatch` 的 `clean` 输入（`true`）跳过 `actions/cache` 按干净机器跑；缓存只覆盖**项目内**路径
（`platform/ci/cargo`：借用 runner 自带 Rust 时的 `CARGO_HOME`，含 registry / git / 工具；以及 `target/debug`），
**不缓存 `target/` 根**（报告与日志必须来自本次运行）。缓存身份包含 `path`，改路径即新缓存项、旧项自动淘汰，首次冷启动重下一次。
CI 借 runner 自带 Rust（`CARGO_HOME` / `SOLOMNI_CARGO_HOME` 指项目内），不跑 `rustup` 安装——不写项目外。
供应链工具在进门前预编译装（工具获取策略见 [quality-isolation.md](quality-isolation.md)）；
安装用时经 `SOLOMNI_CI_PREINSTALL_MS` 带进门禁报告，在 `steps` 里以「CI 前置：供应链工具安装」出现——安装不在 `run-tests.js` 内，只能这样进报告。

**为什么必须有 CI**——下面这些结论本地拿不到：

| 场景 | 本地为什么不够 | CI 提供什么 |
| --- | --- | --- |
| 平台专属代码（`capabilities/tools/detail/confine/` 各平台文件、`tests/<平台>/`） | 平台目标的 `main.rs` 首行是 `#![cfg(target_os = …)]`：非本平台的目标整目标为空，代码根本不编译 | 三平台各编译并各跑一次 |
| 真机围栏（ACL / 容器 profile / Landlock / seatbelt） | 本地默认安全模式会跳过会改本机状态的探针 | 一次性 runner 上真跑，并验撤权与 profile 回收 |
| HTTPS/TLS 出站链路 | 受限环境可能取不到系统 TLS 凭证（判据见 [levels.md](levels.md) 的 T4），本地只能 env-skip | 干净 runner 上真连公网端点 |
| T0 八项（clippy 只编译当前平台的 `#[cfg]`、依赖图随平台变；供应链要联网装工具） | 本机只能代表本平台（判的是**平台差异**，不是「改了门禁就要派发」） | 三平台各自零容忍跑一遍 |
| 三种语言的模块（python / node / C++）在真进程里跑 | 本机只代表本平台的解释器与编译器 | 三平台各跑一次真工具链路，indexer 现场编译 |
| 发布前验收 | 本地通过 ≠ 三平台通过 | 三平台报告 + 三平台 `TEST-REPORT-ACCEPTED` |

**什么时候派发、什么时候不派发**：CI 的唯一价值是给出**本机拿不到的结论**（其它平台能不能编译通过、真机围栏与解释器链路、TLS 出站）。判据只有一条——
这次改动的验收结论**是否依赖本机之外**：

- **依赖** → 派发一次，并按下面的读法比对 `sha`；
- **不依赖** → 不派发，本地入口就是验收结论。明确不算理由的：纯逻辑、文档、当前平台的用例，以及**门禁与卫生工具自身的改动**
  （本机跑同一份入口即同一判据）；攒批，等下一次真需要他平台或真机结论时一起派发。
- 用户明确要求派发时例外。

**触发（需要 CI 时）**：只能手动派发，用 `gh` 发起，`--ref` 指向要验证的分支（`main` 禁止直接 push，通常是 `development`）：

```text
gh workflow run test.yml --ref <分支>
```

按干净机器跑（跳过缓存）时加 `-f clean=true`。触发后按下面的读法比对 `sha`。

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

> `envSkips` 只收集 **`[探针]` 前缀**的行：诊断输出一律用 `[诊断]`，别用 `[探针]`，否则会被算成"跳过"。
>
> 判「环境降级」只看**被测试进程**的令牌与行为：`whoami /groups` 里找包 SID 组在现代 Windows 上是**假阴性**
> （包 SID 在令牌的 `TokenAppContainerSid` 字段里，不在组列表里），按它判会把生效的容器记成降级。
> 容器是否生效用**行为对照**（授权落点写得进、父目录按名可用、父目录内容不可见），见 [levels.md](levels.md) 的 T4。

**等多久再拉（推荐节奏）**：派发之后**先等 5 分钟**再拉 `ci-report`；若某个平台的 `meta.json` 的 `sha` 还对不上
（这次 run 没结束），**每次再等 2 分钟**重拉一次，直到三平台的 `sha` 都对得上，或确认 run 已失败/取消。
等的是 git 拉取，不是网页轮询——`AGENTS.md` 禁止查网页；时间上拿不准（比如 runner 排队很久）就委托用户拉取。

**发布通道**（由 `tests/ci-publish.mjs` 在 CI 里自助发布，成败都发）：

- Actions 注释：失败逐条 `::error::`（带断言原文），通过一条 `::notice::` 概览——匿名可读，不需要凭据；
- `ci-report` 滚动分支：同一路径每次覆盖，**只留最近一次**（要历史看 Actions 产物 `test-report-<os>`）；
- 现在只有**手动派发的 run** 才发布 `ci-report`（没有 push / PR 触发）；
- 报告发布失败（例如 token 权限不对）只是 `::warning::`，**不影响**测试本身的成败——所以"没读到报告"不等于"测试没过"。

**本地与 CI 的关系**：本地入口是快速反馈；CI 是**跨平台与真机**的最终判据，**不是每次改动的必经关卡**。
本次改动若结论依赖本机之外，则本地 + CI 两边都跑通、且 CI 的 `sha` 对得上才算验收完成；
否则本地入口通过即验收完成（判据见 [gaps-acceptance.md](gaps-acceptance.md)）。

### 当前报告状态

当前实现实际会输出：

- `pass`：步骤完成；
- `fail`：断言或命令失败（硬失败）；
- `quality-fail`：T0 任一项不过（编译 / 结构审查 / 格式 / clippy / 编译告警 / 依赖重复）；
- `env-skip`：工具缺失等环境性跳过（步骤级），与 `envSkips`（探针级的 `[探针]` 行）并列；
- `skip-platform`：当前平台不适用的空平台目标；
- `gap`：入口没有找到应运行的部分。

报告字段：`steps`（逐步骤状态、用时 `ms` 与超预算标记 `slow`——"哪一步慢"要看它，不看感觉）、`envSkips`（探针级跳过）、
`timeouts`（被墙钟超时终止的命令）、`quality`（`failed` / `steps`）、`gaps`（平台缺口账）、
`globalGaps`（`tests/gaps.yaml` 的长期目标）、`failed`（硬失败数）。
`test-fail` 与 `blocked` 这两个更细的状态当前没有实现，也不在计划内——`fail` 与 `gap` 已能如实表达。

## 二、成功标记

固定标记只增不删，改动含义必须同步更新测试设计：

- `FRONTEND-SMOKE-OK`：前端冒烟完成；
- `E2E-OK`：端到端场景完成；
- `TEST-REPORT-OK`：当前入口的运行步骤没有失败；
- `TEST-REPORT-FAIL`：当前入口有硬失败步骤**或** T0 任一项不过（同时以非零退出码暴露）；
- `TEST-REPORT-ACCEPTED`：当前平台缺口账为空；
- `MUTATION-OK` / `MUTATION-FOUND`：突变测试工作流全捕获 / 有未捕获或超时——**只用于调查，不参与验收**。

固定标记不能替代质量门禁，也不能覆盖 `env-skip`、`gap` 或 `quality-fail`。测试入口必须以非零退出码暴露失败。

