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

### 真机入口（本地）

```text
node run-tests.js --fence-live
```

仅允许在一次性 runner、VM 或明确授权的环境使用：它会改本机状态（写目录 ACL、建容器 profile）并创建容器身份。
普通开发机上**不要**开；本地默认安全模式（见 [quality-isolation.md](quality-isolation.md)）。

### CI（GitHub Actions）：跨平台与真机的唯一事实来源

工作流 `.github/workflows/test.yml`，矩阵 `windows-latest / ubuntu-latest / macos-latest`（`fail-fast: false`，
一个平台失败不影响另外两个出结论），`on: push` 与 `pull_request`；每个平台跑 `node run-tests.js --fence-live`。

**为什么必须有 CI**——下面这些结论本地拿不到：

| 场景 | 本地为什么不够 | CI 提供什么 |
| --- | --- | --- |
| 平台专属代码（`adapters/confine/` 各平台文件、`tests/<平台>/`） | 平台目标的 `main.rs` 首行是 `#![cfg(target_os = …)]`：非本平台的目标整目标为空，代码根本不编译 | 三平台各编译并各跑一次 |
| 真机围栏（ACL / 容器 profile / Landlock / seatbelt） | 本地默认安全模式会跳过会改本机状态的探针 | 一次性 runner 上真跑，并验撤权与 profile 回收 |
| HTTPS/TLS 出站链路 | 受限环境可能取不到系统 TLS 凭证（判据见 [levels.md](levels.md) 的 T4），本地只能 env-skip | 干净 runner 上真连公网端点 |
| T0 六项（clippy 只编译当前平台的 `#[cfg]`、依赖图随平台变） | 本机只能代表本平台 | 三平台各自零容忍跑一遍 |
| 三种语言的模块（python / node / C++）在真进程里跑 | 本机只代表本平台的解释器与编译器 | 三平台各跑一次真工具链路，indexer 现场编译 |
| 发布前验收 | 本地通过 ≠ 三平台通过 | 三平台报告 + 三平台 `TEST-REPORT-ACCEPTED` |

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

**等多久再拉（推荐节奏）**：push 之后**先等 5 分钟**再拉 `ci-report`；若某个平台的 `meta.json` 的 `sha` 还对不上
（这次 run 没结束），**每次再等 2 分钟**重拉一次，直到三平台的 `sha` 都对得上，或确认 run 已失败/取消。
等的是 git 拉取，不是网页轮询——`AGENTS.md` 禁止查网页；时间上拿不准（比如 runner 排队很久）就委托用户拉取。

**发布通道**（由 `tests/ci-publish.mjs` 在 CI 里自助发布，成败都发）：

- Actions 注释：失败逐条 `::error::`（带断言原文），通过一条 `::notice::` 概览——匿名可读，不需要凭据；
- `ci-report` 滚动分支：同一路径每次覆盖，**只留最近一次**（要历史看 Actions 产物 `test-report-<os>`）；
- 只有 **push 事件**才发布 `ci-report`；`pull_request` 的结论只能在 Actions 注释里看；
- 报告发布失败（例如 token 权限不对）只是 `::warning::`，**不影响**测试本身的成败——所以"没读到报告"不等于"测试没过"。

**本地与 CI 的关系**：本地入口是快速反馈，CI 是跨平台与真机的最终判据。两边都跑通、且 CI 的 `sha` 对得上，
才算本次验收完成（见 [gaps-acceptance.md](gaps-acceptance.md)）。

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

