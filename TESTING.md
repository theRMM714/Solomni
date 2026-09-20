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

## 二、当前状态

当前仓库已经具备：

- `src/tests/` 中的测试层：`core.rs`（T1 用例，全内存装配）、`doubles.rs`（替身与装配辅助）；
- `tests/cross-platform/` 跨平台集成与端到端测试；
- `tests/windows/`、`tests/linux/`、`tests/macos/` 平台探针（`tests/helpers/probe.rs` 提供共用探针设施）；
- `src/presentation/web/*.smoke.cjs` 前端冒烟测试；
- `tests/gaps.yaml` 与 `tests/<平台>/gaps.yaml` 缺口账；
- `node run-tests.js` 测试汇总入口（`node start.js -test` 是备好环境后的同一入口）；
- `src/adapters/fake_chat.rs` 中的 `FakeChat` 与 `DemoGateway`；
- `src/tests/doubles.rs` 与 `src/tests/core.rs` 中的 `InMemory*`、`FakeCatalog`、`VecSource`、`ScriptGateway`、`RecordingRunner`、`RecordingFence`、`TestPrompts`、`NoopLog` 等测试装配（替身支持失败注入，供 T2 复用）；
- `src/tests/` 中的契约测试（T2）：
  - `ports.rs`（13 个端口的替身语义）、`fakes.rs`（FakeChat / DemoGateway 的独立契约）；
  - `adapters.rs`（8 个文件系统适配器的真实边界 + 本机环回 HTTP 适配器）；
  - `api.rs`（入站契约：命令与事件、生成期间停止立刻生效、错误如实传播、单条命令 panic 不带垮核心）；
  - `intent.rs`（共享意图层：点名 / 归并 / 唯一名 / 动作分发 / 生成中拒绝改配置）；
  - `routes.rs`（HTTP 路由目录 ↔ 处理器 ↔ 文档 ↔ 前端调用四者机器比对；假能力面逐条验成功 / 错误 / 空 / 边界）；
- **入站契约也是契约**：呈现层只依赖 `core::api` 的四个角色接口与事件台（拿不到 `Core`、拿不到任何核心锁），
  所以它能被假实现整体替换——`routes.rs` 的 `FakeOps` 就是这么逐条测路由的。
- T0 质量门禁已并入同一入口：编译与结构审查是硬失败，格式 / clippy / 编译告警 / 依赖重复按 `tests/quality-baseline.yaml` 比对。

当前平台缺口账（`tests/cross-platform/gaps.yaml`、`tests/<平台>/gaps.yaml`）**为空**：三平台围栏机制与整仓测试
已由三平台 CI 真跑通过（Windows AppContainer + 目录 ACL 授权与撤权、Linux Landlock、macOS seatbelt）。

仍未完成的缺口全部记在 `tests/gaps.yaml`（长期目标、已确认但尚未实施的产品/机制缺口都在那里，细则不复述条目内容）。
条目存在 = 尚未完成；补齐后删除条目，不保留完成历史。
全局账**不**影响 `TEST-REPORT-ACCEPTED`：那个标记只看平台与跨平台层的缺口账。

## 三、要做事时读哪一份

专项要求具有强制性；一次工作涉及多个领域时，必读文档累加。

| 要做的事 | 读这一份 |
| --- | --- |
| 判断一个测试属于哪层、放哪、允许与禁止什么、怎么判定 | [docs/testing/levels.md](docs/testing/levels.md) |
| 写或改 Stub / Fake / Mock / Spy / Fixture，或验收 Fake | [docs/testing/doubles.md](docs/testing/doubles.md) |
| 新增端口、新增替身，或核对真实适配器的覆盖范围 | [docs/testing/port-matrix.md](docs/testing/port-matrix.md) |
| 声明测试的资源边界、清理副作用，或处理质量门禁与基线 | [docs/testing/quality-isolation.md](docs/testing/quality-isolation.md) |
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
