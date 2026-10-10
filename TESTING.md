# 测试架构（TESTING）

> 本文是测试领域的**门户与路由**：测试的目的是什么、现在处于什么状态、要做事时该读哪一份细则。
> 层级定义、替身语义、端口矩阵、质量门禁、入口与 CI、缺口账与验收各自只有一份权威，都在 `docs/testing/` 下——
> **门户不复述细则正文**（见 [AGENTS.md](AGENTS.md)「文档分层与同步」）。
>
> 架构约束见 [ARCHITECTURE.md](ARCHITECTURE.md)，模块交付要求见 [MODULE_SPEC.md](MODULE_SPEC.md)，仓库协作约束见 [AGENTS.md](AGENTS.md)。

## 一、测试的目的与硬原则

测试要回答的是"当前实现是否满足可观察的行为契约"，不是"测试数量是否很多"或"代码覆盖率是否好看"。

- 测试记录事实：通过、失败、环境跳过、缺口必须分开。
- 环境不允许执行不等于通过；必须记录 `env-skip` 与原因。
- 尚未实现测试不等于通过；必须进入缺口账。
- 失败必须有可定位证据：断言、输入、观察结果和必要的日志尾部。
- 测试必须可重复、可隔离、可清理，不依赖执行顺序。
- 默认测试不得修改真实用户数据、真实 `.home/`、真实 `session/`、真实权限或外部服务。
- 真实网络、真实密钥、开发者个人配置不能成为默认测试依赖。
- 跨平台测试必须优先使用路径组件、隔离根和平台能力探针，不把某个平台的行为猜测成所有平台的行为。
- 测试替身也属于代码：Fake、Mock、Stub、Spy 和 Fixture 必须有自己的最小验证。
- 质量门禁和业务测试是两类不同事实；质量门禁失败不能被业务测试通过抵消。
- 不为了测试制造无意义的 trait、包装层、公共状态或重复测试；可测性必须服从架构边界。
- **门禁自身也是被测对象**：解析器与清单比对要能自证——一个会静默全绿的门禁比缺工具更危险。

## 二、当前状态

当前仓库已经具备：

- `src/tests/` 中的测试层：**与 `capabilities/` 同构的一个能力一个文件**（`prompt.rs` / `registry.rs` / `llm.rs` / `workspace.rs` / `session.rs` / `slate.rs` / `taskchain.rs`）+ `conductor.rs`（协调业务自己的用例：会话中心与跨能力编排）、`tools/` 与 `collab/`（这两个能力的用例按域分成四个子模块）、`doubles.rs`（端口替身）与 `builders.rs`（测试装配脚手架）；
- `tests/cross-platform/` 跨平台集成与端到端测试；
- `tests/windows/`、`tests/linux/`、`tests/macos/` 平台探针（`tests/helpers/probe.rs` 提供共用探针设施）；
- `src/presentation/web/assets/*.smoke.cjs` 前端冒烟测试；
- `tests/gaps.yaml`（全局）与 `tests/<平台>/gaps.yaml`（平台）缺口账；
- **谁做任务谁补测试**：做完就补上这次改动的用例并跑门禁，通过后同步文档（见 [AGENTS.md](AGENTS.md) 九）；
- `node run-tests.js` 测试汇总入口（`node start.js -test` 是备好环境后的同一入口）；
- `src/capabilities/llm/detail/fake_chat.rs` 中的 `FakeChat` 与 `DemoGateway`；
- `src/tests/doubles.rs`（`InMemory*` / `FakeCatalog` / `VecSource` / `ScriptGateway` / `RecordingFence` / `TestPrompts` / `NoopLog`）与 `src/tests/builders.rs`（`RecordingRunner` / `ParallelRunner` / `SilentRunner` / 原生与截断通道替身 / 造会话与造名单的辅助）里的测试装配（替身支持失败注入，供 T2 复用）；
- `src/tests/` 中的契约测试（T2）：
 - `ports.rs`（14 个端口的替身语义；`Log` 在 `kernel/ports.rs`，见 [docs/kernel/unit-map.md](docs/kernel/unit-map.md)）、
 `fakes.rs`（FakeChat / DemoGateway 的独立契约）；
 - `detail.rs`（8 个文件系统实现的真实边界 + 本机环回 HTTP 适配器）；
 - `api.rs`（入站契约：命令与事件、生成期间停止立刻生效、错误如实传播、单条命令 panic 不带垮核心）；
 - `routes.rs`（HTTP 路由目录 ↔ 处理器 ↔ 文档 ↔ 前端调用 ↔ 演示脚本机器比对；假能力面逐条验成功 / 错误 / 空 / 边界）；
- **入站契约也是契约**：呈现层只依赖各能力的能力接口（见 [docs/presentation/contracts.md](docs/presentation/contracts.md)）与事件台（拿不到 `Core`、拿不到任何核心锁），
 所以它能被假实现整体替换——`routes.rs` 的 `FakeOps` 就是这么逐条测路由的。
- T0 质量门禁已并入同一入口：编译、结构审查、格式、clippy、编译告警、依赖重复、**项目外写（env / 工具链）**零容忍硬失败；
  供应链（`cargo audit` / `cargo deny`）同为硬失败，但工具缺失或取不到 advisory 数据时记 `env-skip`（CI 预编译装，本地不引第三方、走 `cargo install` 到项目内）。
  结构审查里还含**门禁解析器自测**与 **`#[ignore]` 禁令**（报告里 `ignored > 0` 也算失败）；
  项目外写检测在跑完快照 `~/.cargo` / `~/.rustup` 等缓存根，新增即失败，并在启动时清理上次崩溃残留的 `solomni-*` 临时目录（见 [docs/testing/quality-isolation.md](docs/testing/quality-isolation.md)）。
- 默认**串行**跑用例（`--parallel` 只在排查并发/隔离问题时用，见 [tests/gaps.yaml](tests/gaps.yaml) 的 `testing.parallel-flake`）；每步都有墙钟上限，超时即硬失败并把证据记进报告
  `timeouts`；每步用时进报告，超预算标 `[slow]`（只报不拦）。
- `node run-tests.js --coverage` 是**手动覆盖率发现模式**：只用来找盲区，不做通过判据、不设阈值
  （判据与局限见 [docs/testing/execution-ci.md](docs/testing/execution-ci.md)）。
- 突变测试是**独立的手动 CI 工作流** `.github/workflows/mutants.yml`（入口 `tests/ci-mutation.mjs`、
  范围 `tests/mutation-scope.json`）：只有 `core`（小而精）与 `capability`（单能力抽查）两个范围，刻意不做 `all`；
  结果只作调查，**不参与 `TEST-REPORT-ACCEPTED`**，发它自己独占的 `ci-mutation` 滚动分支（各推各的，不碰 `ci-report`），
  完整现场走 Actions 产物。
- 门禁之外另有一个**仓库卫生审查**脚本 `run-hygiene.js`（注释契约存量棘轮 + 内容卫生；报告只报不拦、收紧只能往下，也不进 `TEST-REPORT-*`）：
  判据与用法见 [docs/testing/quality-isolation.md](docs/testing/quality-isolation.md) 的「门禁之外」。

**逐平台**与**跨平台**缺口账（`tests/<平台>/gaps.yaml`、`tests/cross-platform/gaps.yaml`）**都为空**：
Linux Landlock、macOS seatbelt 与 Windows 的目录 ACL 授权/撤权、落点清单都已由三平台 CI 真跑通过。
判「环境不允许」的判据只有一条：**看被测试进程自己的令牌与行为**，不拿二手字符串特征当判据。
- Windows 容器里 `whoami /groups` 不含包 SID 组是**正常**的（包 SID 在令牌的 `TokenAppContainerSid` 字段，
  不在组列表里），按它判「环境降级」是假阴性；容器是否生效由**行为对照**给结论——授权落点写得进、
  从父目录按名走得到叶子、父目录里的其它条目看不到。
- 工作区被写沙箱挡住时，原因不是「受限令牌会话」，而是 DSH 的 Windows 写沙箱后端授写权时给授权根打上的
  Low 完整性标签；判据、后果与处置见 [docs/testing/execution-ci.md](docs/testing/execution-ci.md) 的「真机入口」一节。
- Windows 的「模块目录只读 + `userdata/` 可写 + 另一席不可达」由容器往返探针在真机上验收。

仍未完成的缺口全部记在 `tests/gaps.yaml`（长期目标、已确认但尚未实施的产品/机制缺口都在那里，细则不复述条目内容）。
条目存在 = 尚未完成；补齐后删除条目，不保留完成历史。
全局账**不**影响 `TEST-REPORT-ACCEPTED`：那个标记只看平台与跨平台层的缺口账。

## 三、要做事时读哪一份

专项要求具有强制性；一次工作涉及多个领域时，必读文档累加。

| 要做的事 | 读这一份 |
| --- | --- |
| 判断一个测试属于哪层、放哪、允许与禁止什么、怎么判定 | [docs/testing/levels.md](docs/testing/levels.md) |
| 写或改 Stub / Fake / Mock / Spy / Fixture，或验收 Fake | [docs/testing/doubles.md](docs/testing/doubles.md) |
| 新增端口、新增替身，或核对真实适配器的覆盖范围 | [docs/testing/doubles.md](docs/testing/doubles.md)（端口矩阵在 §三） |
| 声明测试的资源边界、清理副作用，或处理质量门禁 | [docs/testing/quality-isolation.md](docs/testing/quality-isolation.md) |
| 跑本地入口、读报告、认成功标记，或处理 CI 与报告发布 | [docs/testing/execution-ci.md](docs/testing/execution-ci.md) |
| 记缺口、看目录与命名、按验收清单收口 | [docs/testing/gaps-acceptance.md](docs/testing/gaps-acceptance.md) |
| 交付一个模块（模块作者要交什么证据） | [docs/testing/module-delivery.md](docs/testing/module-delivery.md) |

## 四、判据速查（细则为准）

- **缺口的唯一真相**是 `tests/gaps.yaml`（全局）与 `tests/<平台>/gaps.yaml`（平台）；门户与细则都不复述条目内容。
- **成功标记**（`FRONTEND-SMOKE-OK` / `E2E-OK` / `TEST-REPORT-OK` / `TEST-REPORT-FAIL` / `TEST-REPORT-ACCEPTED`）
 的固定含义见 [docs/testing/execution-ci.md](docs/testing/execution-ci.md)。
- **质量失败不能被业务测试通过抵消**：两边的结论分别记，判据见 [docs/testing/quality-isolation.md](docs/testing/quality-isolation.md)。
- **需要 CI 的场景**（平台专属代码、真机围栏、HTTPS/TLS、其它平台的质量分区）本地跑不出结论，
 必须等 CI 并按 [docs/testing/execution-ci.md](docs/testing/execution-ci.md) 的读法核对 `sha`。