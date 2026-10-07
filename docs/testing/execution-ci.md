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

注意：`cargo test` 与 `cargo fmt --check`、`cargo clippy … -D warnings` **都预期全绿**——T0 六项全是零容忍硬失败，没有存量基线。

### 完整本地入口

```text
node run-tests.js
```

当前入口的实际顺序是：

1. `cargo build`（并打印安全模式 / 真机围栏模式说明）；
2. 运行 `solomni --doctor`，记录当前平台和围栏能力；
3. `T0 编译（--all-targets）`（硬失败）；
4. `T0 结构审查`（硬失败：目标登记 / 孤儿测试文件 / 缺口账格式 / 文档链接完整性）；
5. `T0 格式（fmt --check）`（硬失败）；
6. `T0 静态检查（clippy）`（硬失败）；
7. `T0 编译告警`（硬失败）；
8. `T0 依赖重复（cargo tree）`（硬失败）；
9. `L1 单元（--bin solomni）`；
10. 逐个运行 `cargo test --test cross-platform/windows/linux/macos`；
11. 前端冒烟；
12. 存在编排器时运行 L4 端到端（编排器会**现场构建 indexer**：编译器版本进日志，构建失败即失败）；
13. 写入 `target/test-report.json` 并打印 `TEST-REPORT-OK` 或 `TEST-REPORT-FAIL`。

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

**本地这条最后一行有个前提：门禁要在「普通 shell」里跑。** 如果本地 shell 本身是受限令牌
（例如低完整性 / 文件沙箱的会话），三件事会同时不成立，而报错都指向错误的方向：

- `%LOCALAPPDATA%\Python` 之类**用户目录下的解释器**访问被拒 → doctor 报"没有 python"、
  L4 的真工具场景报 `'python' is not recognized`（看着像产品缺陷，其实是环境）；
- **改目录 DACL 被拒**（错误码 5）→ 容器围栏装不上，探针只能 env-skip；
- Node 的**管道 stdio 捕获被拒（EPERM）**→ L4 收尾的围栏回收 `status` 为 null、输出为空，
  被判成"本机留下了没人管的痕迹"；同一条还会让**所有"探一探这个工具在不在"的代码集体误判**：
  L4 现场构建 indexer 报"找不到可用的 C++ 编译器"（其实仓库自带 `.tools/mingw64` 的 g++ 能跑，`stdio: ignore` 下 `status=0`），
  `git` 之类的子进程也一样起不来。要在这种会话里跑子进程，就把 stdout **重定向到文件**再读
  （`stdio: ["ignore", fd, "ignore"]`），别用管道。
  （python 同理：它可能确实装着（例如 `%LOCALAPPDATA%\Python\pythoncore-*\python.exe`），
  但沙箱拒绝读那个目录，连它依赖的 DLL 都读不到 → 进程以 ENOENT 起来 → 看起来像"没装"。）

判据：`whoami /groups` 里出现 `Mandatory Label\Low Mandatory Level` 就是这种会话。
**但要查对进程**：有的受限环境里 shell 自己显示 Medium，而**工作区里的二进制**带 Low 完整性标签
（`icacls <exe>` 打出 `Mandatory Label\Low Mandatory Level:(I)(NW)`，且标签改不动）——于是 cargo 拉起的
rustc / 链接器 / 测试二进制全都以 Low 令牌运行，症状与受限会话一模一样：链接器报
`Cannot create temporary file in …\Temp\: Permission denied`、产品自检报 `建自检目录失败：拒绝访问`、
测试二进制 `CreateAppContainerProfile` 报 `0x80070005`。这种会话要拿到结论，只有把**构建好的二进制复制到
工作区外**再跑，或者换一台不受限的机器。

**这种会话里要拿到可信结论，只有两条路，按优先级：**

1. **换普通 shell（推荐，也是唯一的常规路径）**：在不受限的 PowerShell 窗口里跑同一份门禁
   （`node run-tests.js`；要验真机围栏再加 `--fence-live`，那会改本机状态，只在一次性 runner 或明确授权的机器上做）。
2. **对这一次执行放宽沙箱（提权）**：只能在受限会话里跑时，可为**单次执行**申请放宽到不受限，
   批准范围仅限该次、只用于本来被沙箱拒掉的动作用。两条硬约束：
   - **必须有人批准**：批准不到的会话会一直等着、根本不发车——所以它不是默认路径，也不该写进自动化；
   - 放宽只解决"环境不允许"，**不替代**真机围栏验收：`--fence-live` 仍然只在一次性环境里跑。

> **受限令牌会话里的一切围栏 / L4 结论都不可信**——实测（同一台机器、同一份代码）：
> 在 `Mandatory Label\Low Mandatory Level` 的会话里，doctor 报容器围栏装不上（读写 DACL 都 Error 5）、
> `python` 在工具进程里不可达（`where` 找不到、绝对路径 `Access is denied`）、围栏回收因 Node 管道 stdio
> 被拒（EPERM，`status` 为 null）被判成"本机留下了没人管的痕迹"；
> 换成**普通或提权（`High`）会话**后：`node run-tests.js` 直接 `TEST-REPORT-OK`，L4 `E2E-OK`，
> doctor 报 `fs=true net=true tree=true`（AppContainer + Job Object 内核强制），python / node / C++ 真工具链全跑通。
> **所以"卷不支持 ACL""python 装得不对"这类结论都是误判**——判据只有一个：
> `whoami /groups` 里出现 `Mandatory Label\Low Mandatory Level`，就别信这次的门禁结论。
> 提权与普通 shell 给出同一种结论；`--fence-live`（改本机状态）仍然只在一次性环境里做。

### CI（GitHub Actions）：跨平台与真机的唯一事实来源

工作流 `.github/workflows/test.yml`，矩阵 `windows-latest / ubuntu-latest / macos-latest`（`fail-fast: false`，
一个平台失败不影响另外两个出结论），`on: push` 与 `pull_request`；每个平台跑 `node run-tests.js --fence-live`。

**为什么必须有 CI**——下面这些结论本地拿不到：

| 场景 | 本地为什么不够 | CI 提供什么 |
| --- | --- | --- |
| 平台专属代码（`capabilities/tools/detail/confine/` 各平台文件、`tests/<平台>/`） | 平台目标的 `main.rs` 首行是 `#![cfg(target_os = …)]`：非本平台的目标整目标为空，代码根本不编译 | 三平台各编译并各跑一次 |
| 真机围栏（ACL / 容器 profile / Landlock / seatbelt） | 本地默认安全模式会跳过会改本机状态的探针 | 一次性 runner 上真跑，并验撤权与 profile 回收 |
| HTTPS/TLS 出站链路 | 受限环境可能取不到系统 TLS 凭证（判据见 [levels.md](levels.md) 的 T4），本地只能 env-skip | 干净 runner 上真连公网端点 |
| T0 六项（clippy 只编译当前平台的 `#[cfg]`、依赖图随平台变） | 本机只能代表本平台（判的是**平台差异**，不是「改了门禁就要推」） | 三平台各自零容忍跑一遍 |
| 三种语言的模块（python / node / C++）在真进程里跑 | 本机只代表本平台的解释器与编译器 | 三平台各跑一次真工具链路，indexer 现场编译 |
| 发布前验收 | 本地通过 ≠ 三平台通过 | 三平台报告 + 三平台 `TEST-REPORT-ACCEPTED` |

**什么时候该推、什么时候不该推**：CI 的唯一价值是给出**本机拿不到的结论**（其它平台能不能编译通过、真机围栏与解释器链路、TLS 出站）。判据只有一条——
这次改动的验收结论**是否依赖本机之外**：

- **依赖** → 必须推，并按下面的读法比对 `sha`；
- **不依赖** → 不推，本地入口就是验收结论。明确不算理由的：纯逻辑、文档、当前平台的用例，以及**门禁与卫生工具自身的改动**
  （本机跑同一份入口即同一判据）；攒批，等下一次真需要他平台或真机结论时一起推。
- 用户明确要求推时例外。

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

> `envSkips` 只收集 **`[探针]` 前缀**的行：诊断输出一律用 `[诊断]`，别用 `[探针]`，否则会被算成"跳过"（曾把 Windows 的 1 条真跳过记成 5 条）。
>
> 曾经记过一条环境结论——「GitHub 托管的 `windows-latest` 上 AppContainer 会被静默降级（容器内 `whoami /groups`
> 无包 SID 组）」：普通会话的真机查下来**不成立**。包 SID 在令牌的 `TokenAppContainerSid` 字段里、不在组列表里，
> 容器其实生效（真机令牌转储：`TokenIsAppContainer=1`、`AppContainerSid` = 该 profile 的 SID、`capabilities=0`）；
> 那条 `env-skip` 是判据的**假阴性**，现已改成行为对照。
> **教训**：判「环境降级」要看**被测试进程**的令牌与行为，别拿一个二手字符串特征当判据。

**等多久再拉（推荐节奏）**：push 之后**先等 5 分钟**再拉 `ci-report`；若某个平台的 `meta.json` 的 `sha` 还对不上
（这次 run 没结束），**每次再等 2 分钟**重拉一次，直到三平台的 `sha` 都对得上，或确认 run 已失败/取消。
等的是 git 拉取，不是网页轮询——`AGENTS.md` 禁止查网页；时间上拿不准（比如 runner 排队很久）就委托用户拉取。

**发布通道**（由 `tests/ci-publish.mjs` 在 CI 里自助发布，成败都发）：

- Actions 注释：失败逐条 `::error::`（带断言原文），通过一条 `::notice::` 概览——匿名可读，不需要凭据；
- `ci-report` 滚动分支：同一路径每次覆盖，**只留最近一次**（要历史看 Actions 产物 `test-report-<os>`）；
- 只有 **push 事件**才发布 `ci-report`；`pull_request` 的结论只能在 Actions 注释里看；
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

报告字段：`steps`（逐步骤状态与用时 `ms`——"哪一步慢"要看它，不看感觉）、`envSkips`（探针级跳过）、`quality`（`failed` / `steps`）、
`gaps`（平台缺口账）、`globalGaps`（`tests/gaps.yaml` 的长期目标）、`failed`（硬失败数）。
`test-fail` 与 `blocked` 这两个更细的状态当前没有实现，也不在计划内——`fail` 与 `gap` 已能如实表达。

## 二、成功标记

固定标记只增不删，改动含义必须同步更新测试设计：

- `FRONTEND-SMOKE-OK`：前端冒烟完成；
- `E2E-OK`：端到端场景完成；
- `TEST-REPORT-OK`：当前入口的运行步骤没有失败；
- `TEST-REPORT-FAIL`：当前入口有硬失败步骤**或** T0 任一项不过（同时以非零退出码暴露）；
- `TEST-REPORT-ACCEPTED`：当前平台缺口账为空。

固定标记不能替代质量门禁，也不能覆盖 `env-skip`、`gap` 或 `quality-fail`。测试入口必须以非零退出码暴露失败。

