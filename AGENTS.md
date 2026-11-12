# AGENTS.md

> 全仓常驻规则：只放始终生效的约束、请求分类与文档路由；任务细节按需读 `docs/`。

## 一、效力与优先级

- 本文件在全仓生效。
- 与其它文档冲突：以本文件为准，并向用户指出。
- 与用户要求冲突：先说明冲突和影响，用户明确确认后才执行。
- 文档出现逻辑、过时、理论或专业性错误，向用户报告。
- 不隐瞒不确定性：能取证先调查；证据不足且影响结果时直接问用户。
- 开始调查用户请求前，先检查当前会话可用的 skills、MCP 与工具，优先用能直接取证或执行的那一个。
- 只调查与任务高相关的部分，避免信息过载。

## 二、项目不变量

- 使用简体中文交流。
- GREEN FIELD：只维护最新唯一实现；禁止为旧版本、旧资产、已删除实现或假设中的其它仓库保留兼容层、废代码或历史说明。<!-- hygiene-ignore -->
- 不擅自引入版本迁移、版本号规划或兼容策略。
- 工具链与环境用最新稳定版；落后时说明并获授权后更新。
- 默认同时支持 Windows、macOS、Linux；设计先确认跨平台成立。
- 发行版二进制为纯 Rust，用户开箱即用，不装环境。
- git 推送前检查并脱敏；代码、文档、注释不含用户相关信息或与项目无关的内容。

## 三、路径硬约束

- 项目内路径以仓库根为基准，用相对路径；禁止盘符、用户目录等机器绝对路径。
- 代码用路径组件拼接（如 `root.join(".home").join("providers.yaml")`），不把 `/` 或 `\` 写进字符串再拼路径。
- 仓库内文件引用写 `README.md`、`modules/<id>/module.yaml` 等相对形式。
- 依赖、工具链、配置默认收敛在项目目录内；确需项目外目录时先说明原因并获同意。

例外：`.gitignore` 行首 `/` 和 `**/` 的 Git 语义；URL 路径；以产品根为锚、可整体迁移的 `.home/` 与模块 `userdata/`。

## 四、代码与文档约束

- 遵守 DIP，高内聚低耦合，按业务功能垂直切分；业务之间通过 API 契约协作，不依赖其它业务内部实现。
- 任意行为与实现必须通过 `TESTING.md` 规定的适用测试；不可测试的行为记为设计或测试缺口。
- 注释只描述当前功能、约束和联动模块；文件头与行内注释按 `ARCHITECTURE.md` 的「注释契约」写（格式由 T0 判，内容卫生另报）。
- 长期文档只写当前状态，不写过程、旧实现复盘或变更历史；历史归 Git、PR、issue、CI。<!-- hygiene-ignore -->
- 根门户只做路由，细则在 `docs/`；一个事实只有一处权威。改文档必须同次同步相关门户、引用与机器比对段，细则见 `docs/conventions.md`。
- 术语以 `docs/conventions.md` 为唯一口径；常用：agent、模块、单元、能力、共享区、转录。
- 引用用仓库相对路径 + 文件/标题，不写深层节号；不生产没有约束、行为或使用价值的文档。

## 五、请求分类

| 类型 | 判据 | 处理 |
| --- | --- | --- |
| 咨询或审查 | 只调查、解释、总结、评审，不修改项目 | 直接取证回答，不建分支 |
| 任务 | 新功能、重构，或改变行为、契约、结构、依赖、数据模型、目录或设计 | 先给整体方案，用户同意后实施 |
| 低级 bug | 拼写、变量、局部笔误或明显边界错误，且不改变接口、架构、行为设计、数据模型、依赖或目录 | 免方案审批；仍先建 Draft PR 再修 |
| 非低级 bug | 涉及架构、设计、接口、数据、并发、安全、权限或依赖 | 先说现象、证据、影响、可能根因与候选方案，由用户决定 |
| 不确定 | 证据不足，或分类会影响修改方式 | 按非低级 bug 处理，先询问用户 |

补充规则：

- 混合请求按最高风险类别处理；涉及真实数据、权限或线上环境先取得用户确认。
- 批准的方案在约定范围内持续有效；范围、接口、数据、安全、依赖或验收变化时重新确认。用户明确指定方案 = 已批准；有疑问先解答。
- 范围外问题不混入本次修改，向用户说明后另行处理。

分类补充规则见 `docs/workflow/README.md`。

## 六、任务流程（短版）

1. 调查：读权威文档、取事实，汇报现状证据、影响、候选方案与待拍板问题。
2. 方案：目标、范围（明确不做）、验收、步骤、风险、文档、CI；跨单元契约先冻结；用户同意后开工。
3. 开工：最新 `main` 切分支 → 空提交 → push → 立刻 Draft PR（正文=方案）；**没有 Draft PR 不写实现代码**。
4. 实施：一个阶段一个提交；每次 push 更新 PR 的清单、阶段记录与测试证据；范围变化先改 PR 并重新确认。
5. 收口：代码任务 `node run-tests.js` 全绿；纯叙述型文档任务 `node run-tests.js --docs` 全绿（只代表文档审查 + 结构审查，`docs/presentation/contracts.md` 仍走全量）；`git status` 干净；交用户确认；确认后销账、改最终 PR 正文；用户 Ready/合并。
6. 本机回路：`node dev.js test|fmt|clippy`（透传 `node dev.js cargo -- <参数>`）；门禁 `node run-tests.js`（纯文档用 `--docs`）；cargo 只经这些入口跑，要在终端直接敲，先按 `env.js` 注入环境（细则见 `docs/testing/execution-ci.md` 的「本机开发回路」）。

单 / 多 agent 由 agent 或用户选择，不设强制路由；单 agent 见 `docs/workflow/single-agent.md`，多 agent 见 `docs/workflow/multi-agent.md`。两者只改分工，不改六步、闸门、PR 与验收。

开工命令、开工自检、阶段定义与 PR 更新节奏见 `docs/workflow/single-agent.md`；完整流程与单 / 多 agent 选择见 `docs/workflow/README.md`。

## 七、Git 与 CI

- `main` 是唯一长期分支，有保护：不直接提交、不 push；一切经任务分支 + PR，合并由用户进行。
- 任务分支从最新 `main` 切；分支名用 `task/<简述>`；合并后删除。
- 提交前检查 `.gitignore`，用 `git status` 确认没有意外跟踪文件或无关修改；不覆盖、删除或提交用户已有且不属于本次范围的修改。
- agent 不自行 Ready、不自行合并。
- CI 触发、报告读取与判定见 `docs/testing/execution-ci.md`。

### 推荐格式

使用中文 commit message，采用约定式提交（Conventional Commits）的中文版：

```text
<类型>(<范围>): <简要描述>

<详细说明（可选）>

<关联信息（可选）>
```

### 类型对照表

| 类型 | 含义 | 示例 |
| --- | --- | --- |
| feat | 新功能 | feat(用户): 新增手机号登录功能 |
| fix | 修复 Bug | fix(支付): 修复微信支付回调重复处理的问题 |
| docs | 文档变更 | docs: 更新 API 接口文档 |
| style | 代码格式 | style: 统一缩进为 2 个空格 |
| refactor | 重构 | refactor(订单): 拆分订单服务，提取公共逻辑 |
| perf | 性能优化 | perf(列表): 虚拟滚动优化长列表渲染性能 |
| test | 测试 | test(auth): 补充登录模块单元测试 |
| chore | 构建/工具 | chore: 升级 Node.js 至 v20 |

## 八、文档路由

专项要求具有强制性；工作涉及多个领域时，必读文档累加。

**先读门户，再按需读细则。** 完整单元与横向索引见 `docs/README.md`。

| 工作内容 | 门户 |
| --- | --- |
| 项目概括和快速开始 | `README.md` |
| 项目概括和快速开始（English） | `README_EN.md` |
| 架构、分层、业务边界判据（§九）、硬要求（R1–R13）、日志、提示词或落盘 | `ARCHITECTURE.md` |
| 产品行为、运行流程与用户旅程 | `PRODUCT.md` |
| 项目方向、承诺与判据 | `PHILOSOPHY.md` |
| 模块开发或修改 | `MODULE_SPEC.md` |
| 模块产物的构建（创建前自动补编、目标环境、缓存、失败） | `docs/builds/README.md` |
| 存储空间、缓存回收与虚拟根还原 | `docs/storage/README.md` |
| 系统工具、角色、路径模型、回报与验收 | `SYSTOOL.md` |
| 待实现的系统工具清单与方案 | `SYSTOOL.md` → `systool_gaps.yaml` |
| 运行包开发或修改 | `RUNTIME_SPEC.md` |
| providers、models、agents、settings 登记处 | `REGISTRY_SPEC.md` |
| 任意测试、测试替身、质量门禁、缺口账或测试报告 | `TESTING.md` |
| 任务流程 / PR / Git（单 / 多 agent 选择） | `docs/workflow/README.md` |
| 单 agent 流程细则 | `docs/workflow/single-agent.md` |
| 多 agent 流程细则 | `docs/workflow/multi-agent.md` |
| 用户启动文档审计 / 治理 | `docs/workflow/docs-audit.md` |
| 文档分层 / 术语 / 引用 | `docs/conventions.md` |
| Rust 源码布局 / 格式 / 静态检查 / unsafe / 依赖 | `docs/rust-dev.md` |
| 全部单元与横向细则 | `docs/README.md` |

新增根 `*.md` 必须进本表；临时工作清单按 `tests/gates/docs/root-router.js` 的白名单处理。

## 九、文档效力与取证

文档是**某个提交上的断言**，不是权威。冲突时按这个顺序取事实（高 → 低）：

1. 可执行事实：测试、命令输出、CI 报告、生成段；
2. 代码与配置（`src/`、`.github/workflows/`、`links.yaml`、`tests/*/gaps.yaml`）；
3. 机器生成段（`<!-- BEGIN:… -->`）；
4. 文档叙述（含本文件与各门户）。

- 文档与事实冲突：以事实为准，**先报告冲突再修文档**；不按文档改代码。
- 文档格式由 `docs/templates/` 锁定（门户 / 规范 / 参考 / 单元 / 表）；`node run-tests.js --docs` 对骨架、重复块、表头、效力行、锚点与路径做硬检查，违规即失败。
- 每条「必须 / 不得」必须带锚点（仓库路径、测试名、命令、缺口 id、生成段）；没有锚点的判断只能写成「原则 / 建议」，不得据此改行为。
- 查事实先取证，不靠读散文：

| 要确认的事实 | 取证 |
| --- | --- |
| `main` 的必填检查 | `gh api repos/<owner>/<repo>/rulesets` |
| 工作流触发 / 跳过 / 汇总 | `.github/workflows/test.yml` |
| 测试入口参数 / 模式 / 车道 / 步骤 / 标记 | `node run-tests.js --list [--json]`（`tests/gates/registry.js`、`tests/gates/steps.js`、`tests/lib/markers.js`） |
| 平台支持与验收状态 | `tests/gaps.yaml`、`tests/<平台>/gaps.yaml`、代码里的 `VERIFIED_HOSTS` |
| 外部链接与工具链版本 | `links.yaml`、`dev-tools.js` |
| 文档与真相源的机器比对 | `tests/gates/structure/`（8 文件）、`tests/gates/docs/`（4 文件）、`node tests/docs-sync.mjs` |
