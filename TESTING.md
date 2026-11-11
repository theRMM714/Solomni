# 测试架构（TESTING）

> 定位：测试领域的门户与路由：目的、当前状态、要做事时读哪一份细则。
> 效力：规范｜核对：2026-11-04｜依据：`run-tests.js`

## 适用与判据
- 任意测试、测试替身、质量门禁、缺口账或测试报告以本文为入口（`TESTING.md`）。
- 门户不复述细则正文；违反由 `node run-tests.js` / `--docs` 的文档审查与结构审查判定。

## 一、测试的目的与硬原则

- 测试要回答的是"当前实现是否满足可观察的行为契约"，不是"测试数量是否很多"或"代码覆盖率是否好看"；覆盖率在这里是风险下限与回归护栏，不是质量目标。
- 风险下限按代码风险区判，不按测试层：产品代码 80%、平台/进程适配 60%、安全策略 100%（`tests/coverage-policy.json`、`tests/coverage.mjs`）。安全的行为矩阵必须 100%（`src/tests/permission.rs`、`src/kernel/detail/confine/tests.rs`）；OS 强制后端归平台桶，由三平台真机探针（`tests/<平台>/`）与突变审计（`tests/mutation-scope.json`）把关。E2E 不设行覆盖率，旅程清单必须全过（`tests/cross-platform/e2e/browser.cjs`）。

- 测试记录事实：通过、失败、环境跳过、缺口必须分开（`run-tests.js`）。
- 环境不允许执行不等于通过；必须记录 `env-skip` 与原因（`run-tests.js`）。
- 尚未实现测试不等于通过；必须进入缺口账（`tests/gaps.yaml`）。
- 失败必须有可定位证据：断言、输入、观察结果和必要的日志尾部（`target/test-report.json`）。
- 测试必须可重复、可隔离、可清理，不依赖执行顺序（`run-tests.js`）。
- 默认测试不得修改真实用户数据、真实 `.home/`、真实 `session/`、真实权限或外部服务。
- 真实网络、真实密钥、开发者个人配置不能成为默认测试依赖。
- 跨平台测试必须优先使用路径组件、隔离根和平台能力探针，不把某个平台的行为猜测成所有平台的行为（`tests/helpers/probe.rs`）。
- 测试替身也属于代码：Fake、Mock、Stub、Spy 和 Fixture 必须有自己的最小验证（`src/tests/ports.rs`）。
- 质量门禁和业务测试是两类不同事实；质量门禁失败不能被业务测试通过抵消。
- 不为了测试制造无意义的 trait、包装层、公共状态或重复测试；可测性必须服从架构边界（`ARCHITECTURE.md`）。
- **门禁自身也是被测对象**：解析器与清单比对要能自证——一个会静默全绿的门禁比缺工具更危险。

## 二、当前状态

- 测试分区：`src/tests/` 与 `capabilities/` 同构（一个能力一个文件，用例多的按域分子模块；`doubles.rs` 端口替身、`builders.rs` 装配脚手架），跨平台集成与 L4 在 `tests/cross-platform/`，平台探针 `tests/<平台>/`，前端冒烟 `src/presentation/web/assets/*.smoke.cjs` 与前端类型检查（`frontend-typecheck` 步骤，根 `tsconfig.json` + `package-lock.json` 固定 TypeScript）（索引见 [docs/testing/README.md](docs/testing/README.md)）。
- 入口：`node run-tests.js`（`node start.js -test` 是备好环境后的同一入口）；纯叙述型文档改动用 `--docs`；`--coverage`、突变工作流、真机 `--fence-live` 等手动模式见 [docs/testing/execution-ci.md](docs/testing/execution-ci.md)；机器契约 `docs/presentation/contracts.md`（ROUTES 表由 Rust 测试消费）仍走全量。
- T0 九项已并入同一入口（`run-tests.js`）：步骤、可行状态与环境跳过口径以 [docs/testing/levels.md](docs/testing/levels.md) 的 T0 生成段为准（`env-skip` 不算通过）；查清单用 `node run-tests.js --list`。
- 缺口账：全局 `tests/gaps.yaml`、跨平台 `tests/cross-platform/gaps.yaml`、平台 `tests/<平台>/gaps.yaml`；条目、数量与状态一律以账本为准（门户不复述，也不保留完成历史），`TEST-REPORT-ACCEPTED` 只看平台与跨平台账（`run-tests.js`）；格式与验收见 [docs/testing/gaps-acceptance.md](docs/testing/gaps-acceptance.md)。
- 判「环境不允许」只看被测试进程自己的令牌与行为，不拿二手字符串特征当判据；Windows 容器与写沙箱的判据、后果与处置见 [docs/testing/execution-ci.md](docs/testing/execution-ci.md) 的「真机入口」。
- 门禁之外的仓库卫生由 `node run-hygiene.js` 报（注释契约棘轮 + 内容 / 语义卫生；不进 T0、不进 `TEST-REPORT-*`、不改成败），见 [docs/testing/quality-isolation.md](docs/testing/quality-isolation.md) 的「门禁之外」。
- 谁做任务谁补测试：同次提交补用例、跑门禁，收口时同步文档（`AGENTS.md` 六）。

## 三、要做事时读哪一份

- 专项要求具有强制性；一次工作涉及多个领域时，必读文档累加。

| 要做的事 | 读这一份 |
| --- | --- |
| 判断一个测试属于哪层、放哪、允许与禁止什么、怎么判定 | [docs/testing/levels.md](docs/testing/levels.md) |
| 写或改 Stub / Fake / Mock / Spy / Fixture，或验收 Fake | [docs/testing/doubles.md](docs/testing/doubles.md) |
| 新增端口、新增替身，或核对真实适配器的覆盖范围 | [docs/testing/doubles.md](docs/testing/doubles.md)（端口矩阵在 §三） |
| 声明测试的资源边界、清理副作用，或处理质量门禁 | [docs/testing/quality-isolation.md](docs/testing/quality-isolation.md) |
| 跑本地入口、读报告、认成功标记，或处理 CI 与报告发布 | [docs/testing/execution-ci.md](docs/testing/execution-ci.md) |
| 记缺口、看目录与命名、按验收清单收口 | [docs/testing/gaps-acceptance.md](docs/testing/gaps-acceptance.md) |
| 交付一个模块（模块作者要交什么证据） | [docs/testing/module-delivery.md](docs/testing/module-delivery.md) |
| 看测试与质量细则的目录索引 | [docs/testing/README.md](docs/testing/README.md) |

## 四、判据速查（细则为准）

- **缺口的唯一真相**是 `tests/gaps.yaml`（全局）与 `tests/<平台>/gaps.yaml`（平台）；门户与细则都不复述条目内容。
- **成功标记**的固定清单与含义以 [docs/testing/execution-ci.md](docs/testing/execution-ci.md) 的生成段为准（唯一声明在 `tests/lib/markers.js`，`node run-tests.js --list` 可查）；注意 `TEST-REPORT-OK` 在 `--docs` / `--audit` 下只代表对应审查通过，CI 报告统一写 `ci-report` 的 `runs/structure` / `runs/docs` / `runs/<os>`。
- **质量失败不能被业务测试通过抵消**：两边的结论分别记，判据见 [docs/testing/quality-isolation.md](docs/testing/quality-isolation.md)。
- **需要 CI 的场景**（平台专属代码、真机围栏、HTTPS/TLS、其它平台的质量分区）本地跑不出结论，必须等 CI 并按 [docs/testing/execution-ci.md](docs/testing/execution-ci.md) 的读法核对 `sha`。

## 检查
- 测试目的与硬原则 → 评审判断（无机器判据）。
- 缺口账 → `tests/gaps.yaml`（全局）与 `tests/<平台>/gaps.yaml`（平台）。
- 文档结构 → `node run-tests.js --docs` 的文档审查。

## 路由
- 测试层级与判定：[docs/testing/levels.md](docs/testing/levels.md)
- 测试替身：[docs/testing/doubles.md](docs/testing/doubles.md)
- 隔离、清理与质量门禁：[docs/testing/quality-isolation.md](docs/testing/quality-isolation.md)
- 执行入口、报告与 CI：[docs/testing/execution-ci.md](docs/testing/execution-ci.md)
- 缺口账与验收：[docs/testing/gaps-acceptance.md](docs/testing/gaps-acceptance.md)
- 模块测试交付：[docs/testing/module-delivery.md](docs/testing/module-delivery.md)
- 测试细则索引：[docs/testing/README.md](docs/testing/README.md)
- 架构约束：[ARCHITECTURE.md](ARCHITECTURE.md)
- 仓库协作约束：[AGENTS.md](AGENTS.md)
