# 目录、缺口账与验收

> 定位：测试目录与命名、缺口账格式、工作方法与验收清单的唯一权威。
> 效力：规范｜核对：2026-11-28｜依据：`tests/gaps.yaml`

## 适用与判据
- 记缺口、定目录与命名、按验收清单收口时以本文为准（`docs/testing/gaps-acceptance.md`）。
- 违反由 T0 结构审查（缺口账格式、孤儿测试；`tests/gates/structure/ledgers.js` / `targets.js`）与 CI 判定。

## 一、目录、目标与命名

- 顶层分区：
  - `tests/cross-platform/`：T3 集成、T5 端到端与跨平台缺口账（`tests/cross-platform/gaps.yaml`）。
  - `tests/<平台>/`：`windows` / `linux` / `macos` 真机探针与各自缺口账。
  - `tests/lib/`：入口与门禁的共享实现；`tests/gates/`：门禁清单与判据实现。
  - 根 `tests/*.mjs`：CI / 文档 / 工具自检脚本；逐文件清单以目录与 `node run-tests.js --list` 为准，文档不复述。

- 单元层与 `units/` **同构**：T1 用例按业务分文件——`taskchain.rs` / `prompt.rs` / `registry.rs` / `llm.rs` / `workspace.rs` / `session.rs` / `slate.rs` / `modules.rs` / `packages.rs` / `vm.rs` / `permission.rs` / `residents.rs` / `secrets.rs`，加**协调业务**自己的 `runtime.rs` 与 `runtime/`（`proxy.rs`）。
- 用例多的单元按域分目录（如 `tools/`、`session/`、`workspace/`）；契约测试 `ports.rs` / `fakes.rs` / `detail.rs` / `mcp.rs` / `api.rs` / `cli.rs` / `routes.rs`；横向用例 `diagnostics.rs`（隐藏模式契约）/ `fence_reach.rs`（模块可达范围）。
- 共享支撑 `doubles/`（端口替身，按端口族一个文件）/ `builders/`（测试装配脚手架，按用途一个文件）/ `prelude.rs`（公共前置重导出）/ `tests.rs`。
- 文件粒度（判据 `tests/gates/structure/size.js`）：用例文件 ≤ 2000 行（软线 1500）、支撑文件 ≤ 1200 行（软线 800）；超线就按"用例族 / 端口族"拆子目录，拆分只搬代码，不改用例名与断言。
- **判据**：用例落到"它钉住的那个不变式"所属的文件；顺手经过别的单元只是路径，不是归属。

- 四个平台目标在 `Cargo.toml` 中显式登记。新增测试目标、Fixture 或脚本必须能从入口追溯到执行位置，否则属于结构质量问题。

- 命名要求：

- 测试名称描述行为和条件，不描述实现细节；
- Fake/Stub/Mock/Spy 名称表达职责；
- Fixture 名称表达场景；
- 缺口 id 稳定、唯一、可在报告中引用；
- 平台专属行为放平台目录，跨平台行为放 `cross-platform`；
- 测试报告和日志写入 `target/`，不入库。

## 二、缺口账

### 全局缺口

- `tests/gaps.yaml` 记录任意层级的跨平台测试缺口与**长期目标**（存量收敛），以及**已确认但尚未实施的产品/机制缺口**（例如 ACP 适配器）。平台机制或平台专属验收缺口记在 `tests/<platform>/gaps.yaml`，不重复进全局账。
- **条目要能判定**：写清现状、怎么补、验收标准与为什么还没做；只有"大方向已定但推迟"也要把方向和推迟理由写进去，否则它只是一句没有验收路径的待办。
- 它不参与 `TEST-REPORT-ACCEPTED` 判定，但每条都会以 `[global-gap]` 进报告与 `report.globalGaps`。

### 平台缺口

- `tests/<platform>/gaps.yaml` 只记录平台机制或平台专属验收缺口。条目存在表示当前未完成，不得留下"已完成"的残条；平台账非空时，该平台的 `TEST-REPORT-ACCEPTED` 不成立。

### 缺口格式

```yaml
- id: fake-chat.contract-tests
  scope: global
  level: T2
  why: 说明为什么该行为是必须验证的契约
  how: |
    写出可直接执行的命令或实现步骤
  accept: 可判定的通过条件
  blocked_by: |
    一句话写清阻塞原因（如 无 / 平台不可用 / 环境不允许 / 缺少观察面 / 缺少实现）
```

- 每条缺口必须有：稳定 id、范围、层级、必要性、执行方法、验收条件和阻塞原因。补齐后删除条目，不保留完成历史（`tests/gaps.yaml`）。

## 三、给开发者与 agent 的工作方法

1. 先阅读 [TESTING.md](../../TESTING.md)（门户与路由）、[architecture/README.md](../architecture/README.md) 和相关模块契约。
2. 先判断测试属于 T0-T5 哪一类，不要把质量检查写成业务测试。
3. 优先用工具取得事实：运行入口、查看报告、检查日志和 `gaps.yaml`。
4. 纯逻辑先写 T1；端口替身和真实适配器补 T2；真实边界补 T3；平台机制补 T4；完整旅程补 T5。
5. 新增 Fake 时，同时增加 Fake 的最小契约测试和失败注入说明。
6. 新增测试时检查是否已有等价 Fixture、Spy 或端口契约，避免复制粘贴。
7. 失败时修代码或测试；环境不允许时记录 `env-skip`；未实现时建立 `gap`；不要把任何一种写成通过。
8. 运行完整入口并阅读 `target/test-report.json`，确认报告与日志能解释结果。
9. 测试完成后检查 `git status` 和 `.gitignore`，确保没有测试产物被跟踪。
10. 需要 CI 的场景（见本文「验收清单」）：PR 标 Ready 后自动跑；Draft 阶段或没有 PR 时，按 [execution-ci.md](execution-ci.md) 用 `gh` 手动派发。拉 `ci-report` 后**先比对 `sha`**，再按平台核对 steps / envSkips。
11. 只有当目标行为、质量门禁、当前平台缺口与（需要时）CI 三平台结论都符合要求，才可宣称本次测试验收完成。

## 四、验收清单

- 一次测试相关修改至少要回答：

- 测的是什么行为，属于哪个 T 层？
- 是否有成功、失败、边界和资源清理断言？
- 是否依赖 Fake、Mock、Stub、Spy 或 Fixture？它们是否被单独验证？
- 是否需要真实文件、网络、进程、权限或平台探针？
- 测试是否会修改本机状态？如果会，如何隔离和回收？
- 测试是否被入口实际执行？
- 本次改动落在哪个平台上：只在当前平台可判，还是需要 CI？需要 CI 时，`ci-report` 的 `sha` 是否已经对得上本次提交？
- 失败、跳过、缺口和质量失败能否在报告中区分？
- 是否引入了重复测试、重复 Fixture、重复依赖或无意义抽象？
- 相关 `gaps.yaml` 是否更新为当前状态？
- 文档、报告和日志是否只使用项目相对路径？

- 未能回答的问题不是"以后再说"，而是测试设计或观察面仍不完整，应进入缺口账。

- **需要 CI 才算验收的场景**（其余按 [execution-ci.md](execution-ci.md) 的 CI 表）：改了平台专属代码（`kernel/detail/confine/` 或 `tests/<平台>/`）、改了平台围栏机制、改了 HTTPS/TLS 链路、改了只在其它平台编译的 `#[cfg]` 分支——**这些本机跑不出结论，必须走 CI（Ready 的 PR 自动触发，或手动派发）并比对 `sha`**（`.github/workflows/test.yml`）。

- 反过来同样要守：**本机能跑出结论的改动不必为此额外手动派发 CI**。纯逻辑、文档、当前平台的用例，以及**质量门禁与卫生工具自身的改动**（本机跑的就是同一份 `node run-tests.js` / `run-hygiene.js`，本地入口即判据）都不构成额外派发的理由；PR 标 Ready 时照跑一次，按结果读，但不把 `env-skip` 当通过。

## 检查
- 缺口账格式 → T0 结构审查（`node run-tests.js`）。
- 平台缺口 → `TEST-REPORT-ACCEPTED` 与 `tests/<平台>/gaps.yaml`。

## 路由
- 入口与 CI：[execution-ci.md](execution-ci.md)
- 测试层级与判定：[levels.md](levels.md)
- 测试门户：[../../TESTING.md](../../TESTING.md)
