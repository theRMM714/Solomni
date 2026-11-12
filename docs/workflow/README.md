# 任务与开发流程

> 定位：任务流程总入口：单 / 多 agent 两种工作流、共同六步与闸门、分支与 PR 规则。
> 效力：规范｜核对：2026-11-13｜依据：`AGENTS.md`

## 适用与判据
- 所有任务期操作先读本文，再按选定的工作流读 [single-agent.md](single-agent.md) 或 [multi-agent.md](multi-agent.md)。
- 工作流由 agent 或用户选择，本文不设强制路由判据；两种工作流共用这里的六步、闸门、分支、PR 与验收。
- 违反由 PR 检查与 `node run-tests.js` 判定；agent 不自行 Ready / 合并（`AGENTS.md`）。

## 一、请求分类

- 分类判据见 `AGENTS.md` §五。任务期补充规则：

- 混合请求按最高风险类别处理。
- 涉及真实数据、权限或线上环境时，无论是否看似低级，都先取得用户确认。
- 用户批准的方案在约定范围和验收标准内持续有效，不重复请求批准；范围、接口、数据、安全、依赖或验收变化时，必须重新确认（`AGENTS.md`）。
- 用户发言无疑问并明确要求执行某个具体方案时，视为该方案已获批准；有疑问先解答，确认无意义后再执行。
- 开发中发现不属于当前范围的新问题，不混入本次修改；向用户说明并另行处理。

## 二、工作流选择

- 单 agent：一个 agent 串行完成调查、方案、实施与收口；没有并行协调成本。
- 多 agent：多个 agent 并行调查或并行实施，由一个 Lead 汇总证据、集成改动并对最终结果负责。
- 选择由 agent 或用户决定，本文不设适用门槛；下面只列常见考虑，不作为判定条件：
  - 调查面宽、问题彼此独立时，多 agent 并行取证可以减少串行等待。
  - 实施可以拆成写范围不相交的分片时，多 agent 可以并行推进。
  - 任务小、契约未稳定或需要频繁跨范围决策时，单 agent 通常更省协调。
- 选择结果写进 PR 的「工作流」节；中途切换先改 PR 说明，已完成提交保留。
- 多 agent 不改变请求分类：咨询 / 审查仍然只调查、不建分支（`AGENTS.md`）。

## 三、共同六步与闸门

```text
分类 → 调查 → 方案 → [闸门 1：用户同意]
     → 开工（分支 + 空提交 + push + Draft PR + 方案正文）
     → 实施（阶段提交 + PR 更新）
     → 收口 → [闸门 2：全绿 + 用户确认]
     → Ready / 合并（用户执行）
```

| 步骤 | 必须完成 | 进入下一步的证据 |
| --- | --- | --- |
| 分类 | 判断咨询 / 任务 / bug | 咨询到此为止；任务与非低级 bug 进入调查 |
| 调查 | 读权威文档、用工具取事实、向用户汇报 | 现状证据、影响、候选方案、待拍板问题 |
| 方案 | 目标、范围（含明确不做）、验收、步骤、风险、文档、CI | 用户同意（闸门 1）；跨单元契约冻结 |
| 开工 | 分支、空提交、push、Draft PR、方案正文、报 URL | Draft PR 已存在且正文含方案；未满足不写实现代码 |
| 实施 | 阶段提交；任务分支每次 push 后更新 PR | 阶段代码 + 测试 + 文档已同提交，PR 已更新 |
| 收口 | 全量门禁；差异自查 | 全绿、工作树干净、用户确认（闸门 2） |
| Ready / 合并 | 用户确认后销账、同步文档、改最终 PR 正文 | 用户执行 Ready / 合并；agent 不自行做 |

- 每一步完成后才能进入下一步；闸门不通过就停下报告，不跳步。
- 方案是任务期记录：写进 PR 正文，不写进长期文档；长期文档只在收口时改成当前状态。

## 四、共同分支与 PR

1. 一个任务一个分支：从最新 `main` 切 `task/<简述>`；`main` 有分支保护，不直接提交、不 push（`AGENTS.md`）。
2. 空提交 + push + Draft PR 是写实现前的固定动作；正文按 [PR 模板](../../.github/pull_request_template.md) 写全。
3. 一个任务一个 PR；多 agent 的子分支只作集成来源，不单独开 PR、不合并到 `main`（`docs/workflow/multi-agent.md`）。
4. 每个阶段：代码 + 测试 + 文档同提交；任务分支每次 push 后更新 PR 的清单、阶段记录与测试证据。
5. Draft 期不跑 CI；本机按所选工作流的回路迭代；Ready 后触发验收（`docs/testing/execution-ci.md`）。
6. 收口：全量门禁、差异自查、用户确认（闸门 2）；Ready / 合并由用户执行（`AGENTS.md`）。

## 五、职责边界

- 单 agent：全部调查、写范围、分支、PR、门禁由同一个 agent 负责（[single-agent.md](single-agent.md)）。
- 多 agent：Lead 负责方案、契约、分片、共享文件、集成、PR 与最终门禁；worker 只负责分片与分片自测（[multi-agent.md](multi-agent.md)）。
- 两种工作流都由用户保留闸门 1、闸门 2 与合并权（`AGENTS.md`）。

## 六、例外

- 咨询 / 审查：只出结论，不建分支；可以用多 agent 并行取证（`AGENTS.md`）。
- 文档审计 / 治理：用户启动时按 [docs-audit.md](docs-audit.md) 执行，不套用编程任务的实施节奏。

## 七、Git 与 CI

- 硬规则见 `AGENTS.md` §七：`main` 保护、任务分支、agent 不自行 Ready / 合并。
- 需要他平台或真机结论时，用 `gh workflow run test.yml --ref <分支>` 手动派发；触发时机、报告读取与 `sha` 比对以 [execution-ci.md](../testing/execution-ci.md) 为准。
- 纯文档 PR 在 Ready 后走 `changes` + `docs` + `structure` + 合并闸门；平台 job 跳过；规则集与合并判定见 [execution-ci.md](../testing/execution-ci.md) 的「规则集」。

## 检查

- 流程闸门 → 用户确认与 Draft PR 正文（`.github/pull_request_template.md`）。
- 收口门禁 → `node run-tests.js`（纯文档 `node run-tests.js --docs`）。
- 文档模板与锚点 → `node run-tests.js --docs`（`docs/templates/index.json`）。

## 路由

- 单 agent 细则：[single-agent.md](single-agent.md)
- 多 agent 细则：[multi-agent.md](multi-agent.md)
- 文档审计：[docs-audit.md](docs-audit.md)
- 全仓常驻规则：[../../AGENTS.md](../../AGENTS.md)
- CI 报告细节：[../testing/execution-ci.md](../testing/execution-ci.md)
- 文档索引：[../README.md](../README.md)
- PR 模板：[../../.github/pull_request_template.md](../../.github/pull_request_template.md)
