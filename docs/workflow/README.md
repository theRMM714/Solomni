# 任务与开发流程

> 定位：任务流程总入口：工作流选择、两种工作流共用的约定、开工与文档审计。
> 效力：规范｜核对：2026-11-13｜依据：`AGENTS.md`

## 适用与判据

- 任务期操作先读本文，再按选定的工作流读 [single-agent.md](single-agent.md) 或 [multi-agent.md](multi-agent.md)。
- 请求分类、Git 与 CI 约束以 [AGENTS.md](../../AGENTS.md) 为准；本文只写两种工作流共用的约定。
- 工作流由 agent 或用户选择，本文不设强制路由判据；选择结果写进 PR 的「工作流」节。

## 一、工作流选择

- 单 agent：一个 agent 串行完成调查、方案、实施与收口；没有并行协调成本。
- 多 agent：多个 agent 并行调查或并行实施，由一个 Lead 汇总、集成并对最终结果负责。
- 选择由 agent 或用户决定，本文不设适用门槛；常见考虑：
  - 调查面宽、问题彼此独立时，多 agent 并行取证可以减少串行等待。
  - 实施能拆成写范围不相交的分片时，多 agent 可以并行推进。
  - 任务小、契约未稳定或需要频繁跨范围决策时，单 agent 通常更省协调。
- 中途切换先改 PR 的「工作流」说明，已完成提交保留。

## 二、共同约定

- 一个任务 = 一个任务分支 + 一个 Draft PR；开工先于实现，步骤见下节。
- 子分支只作集成来源，不单独开 PR；集成完成后删 worktree 与子分支。
- 一个阶段 = 一个提交（代码、测试与文档放在一起）；每次 push 更新 PR 的实施清单、阶段记录与验收证据。
- 闸门 1 = 用户同意方案；闸门 2 = 收口全量门禁通过且用户确认。Ready 与合并由用户执行。
- PR 是任务进度的唯一落点：决定、证据与 CI 结论记在 PR 正文或评论。
- 与 `AGENTS.md` 冲突时以 `AGENTS.md` 为准。

## 三、开工：分支 + Draft PR

- 两种工作流共用同一开工步骤，顺序固定：
  1. 从最新 `main` 切任务分支：
     ```sh
     git fetch origin main
     git switch -c task/<简述> origin/main
     ```
  2. 空提交占位并推送：
     ```sh
     git commit --allow-empty -m "chore: 开工 <任务>"
     git push -u origin <分支>
     ```
  3. 创建 Draft PR，正文按 [PR 模板](../../.github/pull_request_template.md) 写全（目标与范围、明确不做、验收标准、实施清单、风险）：
     ```sh
     gh pr create --draft --base main --head <分支>
     ```
  4. 开工自检，全部为是才写实现代码：
     - [ ] 分支从最新 `main` 切出并 push
     - [ ] 空提交已推送
     - [ ] Draft PR 已创建
     - [ ] PR 正文含方案 / 验收 / 实施清单 / 明确不做
- 任一项为否：停下修，不写实现代码（`.github/pull_request_template.md`）。
- 无法创建或访问 PR（无远程、无权限、网络不通）：停下报告，不先干完再补（`AGENTS.md`）。
- Draft 期不跑 CI：本机按所选工作流迭代；需要他平台结论时手动派发（见 [execution-ci.md](../testing/execution-ci.md) 的「CI（GitHub Actions）」）。

## 四、文档审计 / 治理

- 用户想要文档审计 / 治理时按 [docs-audit.md](docs-audit.md) 执行，不套用编程任务的实施节奏。

## 检查

- 工作流选择 → PR 正文的「工作流」节（[.github/pull_request_template.md](../../.github/pull_request_template.md)）。
- 开工自检 → 本文「三、开工：分支 + Draft PR」。
- 文档模板与锚点 → `node run-tests.js --docs`（`docs/templates/index.json` 与 `tests/docs-gov.mjs`）。
- 收口门禁 → `node run-tests.js`（纯文档 `node run-tests.js --docs`）。

## 路由

- 单 agent 细则：[single-agent.md](single-agent.md)
- 多 agent 细则：[multi-agent.md](multi-agent.md)
- 文档审计：[docs-audit.md](docs-audit.md)
- 全仓常驻规则：[../../AGENTS.md](../../AGENTS.md)
- CI 报告细节：[../testing/execution-ci.md](../testing/execution-ci.md)
- 文档索引：[../README.md](../README.md)
- PR 模板：[../../.github/pull_request_template.md](../../.github/pull_request_template.md)
