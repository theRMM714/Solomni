# 目录、缺口账与验收

> 本文是**测试目录与命名、缺口账格式、工作方法与验收清单的唯一权威**。
> 入口与 CI 见 [execution-ci.md](execution-ci.md)，层级与判定见 [levels.md](levels.md)。

## 十一、目录、目标与命名

当前测试目录：

```text
tests/
  helpers/
    probe.rs                    # T4 共用探针设施
    https_probe.rs              # T4 HTTPS/TLS 探针（三平台目标共用这一份正文）
  cross-platform/
    main.rs                     # T3 目标入口
    integration/                # T3（含 fence_launcher.rs）
    e2e/                        # T5：假供应商、驱动、编排与隔离根
    gaps.yaml                   # T3 缺口账
  windows/
    main.rs                     # 平台目标入口
    probes/                     # T4
    gaps.yaml
  linux/
    main.rs
    probes/
    gaps.yaml
  macos/
    main.rs
    probes/
    gaps.yaml
  gaps.yaml                     # 全局长期目标（不影响 TEST-REPORT-ACCEPTED）
  quality-baseline.yaml         # T0 存量基线（由 --print-quality-baseline 生成）
  ci-publish.mjs                # CI 报告发布脚本（把三平台报告写入 ci-report 分支）
```

单元层的契约测试在 `src/contract_tests/`（`ports.rs` / `fakes.rs` / `adapters.rs` / `api.rs` / `intent.rs` / `routes.rs`），替身在 `src/tests.rs`；
两者合并进 `src/tests/` 目录是记在 `tests/gaps.yaml` 的长期目标（`tests.layout-migration`）。

四个平台目标在 `Cargo.toml` 中显式登记。新增测试目标、Fixture 或脚本必须能从入口追溯到执行位置，否则属于结构质量问题。

命名要求：

- 测试名称描述行为和条件，不描述实现细节；
- Fake/Stub/Mock/Spy 名称表达职责；
- Fixture 名称表达场景；
- 缺口 id 稳定、唯一、可在报告中引用；
- 平台专属行为放平台目录，跨平台行为放 `cross-platform`；
- 测试报告和日志写入 `target/`，不入库。

## 十二、缺口账

### 全局缺口

`tests/gaps.yaml` 记录 T0、T2、T5 和测试基础设施的跨平台缺口、**长期目标**（存量收敛、目录迁移），
以及**已确认但尚未实施的产品/机制缺口**（例如围栏权限模型待定）。
它不参与 `TEST-REPORT-ACCEPTED` 判定，但每条都会以 `[global-gap]` 进报告与 `report.globalGaps`。

### 平台缺口

`tests/<platform>/gaps.yaml` 只记录平台机制或平台专属验收缺口。条目存在表示当前未完成，不得留下"已完成"的残条。

### 缺口格式

```yaml
- id: fake-chat.contract-tests
  scope: global
  level: T2
  why: 说明为什么该行为是必须验证的契约
  how: |
    写出可直接执行的命令或实现步骤
  accept: 可判定的通过条件
  blocked_by: 无 | 平台不可用 | 环境不允许 | 缺少观察面 | 缺少实现
```

每条缺口必须有：稳定 id、范围、层级、必要性、执行方法、验收条件和阻塞原因。补齐后删除条目，不保留完成历史。

## 十三、给开发者和 AI 的工作方法

1. 先阅读 [TESTING.md](../../TESTING.md)（门户与路由）、[ARCHITECTURE.md](../../ARCHITECTURE.md) 和相关模块契约。
2. 先判断测试属于 T0-T5 哪一类，不要把质量检查写成业务测试。
3. 优先用工具取得事实：运行入口、查看报告、检查日志和 `gaps.yaml`。
4. 纯逻辑先写 T1；端口替身和真实适配器补 T2；真实边界补 T3；平台机制补 T4；完整旅程补 T5。
5. 新增 Fake 时，同时增加 Fake 的最小契约测试和失败注入说明。
6. 新增测试时检查是否已有等价 Fixture、Spy 或端口契约，避免复制粘贴。
7. 失败时修代码或测试；环境不允许时记录 `env-skip`；未实现时建立 `gap`；不要把任何一种写成通过。
8. 运行完整入口并阅读 `target/test-report.json`，确认报告与日志能解释结果。
9. 测试完成后检查 `git status` 和 `.gitignore`，确保没有测试产物被跟踪。
10. 需要 CI 的场景（见本文「验收清单」）：推上去后按 [execution-ci.md](execution-ci.md) 的 CI 读法拉 `ci-report`，**先比对 `sha`**，再按平台核对 steps / envSkips。
11. 只有当目标行为、质量门禁、当前平台缺口与（需要时）CI 三平台结论都符合要求，才可宣称本次测试验收完成。

## 十四、验收清单

一次测试相关修改至少要回答：

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

未能回答的问题不是"以后再说"，而是测试设计或观察面仍不完整，应进入缺口账。

**需要 CI 才算验收的场景**（其余按 [execution-ci.md](execution-ci.md) 的 CI 表）：改了平台专属代码（`adapters/confine/` 或 `tests/<平台>/`）、
改了平台围栏机制、改了 HTTPS/TLS 链路、改了 `tests/quality-baseline.yaml` 的其它平台分区、
或改了只在其它平台编译的 `#[cfg]` 分支——这些本地跑不出结论，必须等 CI 并比对 `sha`。
