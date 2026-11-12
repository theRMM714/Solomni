# 多 agent 工作流

> 定位：多个 agent 并行调查或并行实施，一个 Lead 汇总与集成；分片、worktree、任务板、集成与清理细则。
> 效力：规范｜核对：2026-11-13｜依据：`docs/workflow/README.md`

## 适用与判据
- 选择多 agent 时按本文执行；共同六步、闸门、分支与 PR 规则见 [README.md](README.md)。
- 本文不设适用门槛；调查面宽、问题彼此独立或实施可拆成不相交写范围时，都可以用多 agent。
- 一个任务只有一个 Lead、一个任务分支、一个 PR；worker 只对分片负责（`docs/workflow/README.md`）。
- 多 agent 不改变请求分类与验收：咨询 / 审查仍只调查，实施仍走同一套门禁（`AGENTS.md`）。

## 一、角色与职责
- Lead：对用户单点负责；定方案、冻结跨分片契约、拆分片、维护任务板、管共享文件、集成、跑最终门禁、维护 PR。
- worker：只改自己的分片写范围，跑分片验收，向 Lead 报告提交、文件、测试与例外。
- 共享文件：基线 / 棘轮、门禁脚本、登记处、文档门户与路由、PR 正文、跨分片契约文件；这些由 Lead 独占（`docs/workflow/README.md`）。
- worker 发现共享文件需要改：停下报告，由 Lead 决定改法或重新分片（`docs/workflow/multi-agent.md`）。
- 用户保留闸门 1、闸门 2 与合并权；Lead 不代替用户确认或合并（`AGENTS.md`）。

## 二、调查型并行
- 适用于调查面宽、多个问题彼此独立，或需要多份证据互相校验的阶段；调查本身也是多 agent 的正式用法。
- Lead 先列问题清单、每个 worker 的范围与要回答的问题，避免重复取证。
- worker 只读取证，不写文件；结论带路径、命令、输出与不确定点。
- 调查型不建 worktree、不建子分支，也不改变「咨询 / 审查不建分支」的规则（`AGENTS.md`）。
- Lead 汇总、去重、标出冲突证据；需要实施时再按实施型拆分片（`docs/workflow/multi-agent.md`）。

## 三、实施型分片与契约
- 每片写清目标、写范围（文件 / 目录）、验收命令、依赖与产物；一个文件只有一个 owner。
- 跨分片契约（API、端口、数据模型、错误文案、盘上格式）由 Lead 先冻结；worker 改契约前先报 Lead。
- 分片以「可独立验收」为准，不按固定人数或工时硬拆；拆到需要频繁跨片协调时改回单 agent 或重排。
- 有任务板时登记分片、`writeScopes`、`blocked_by` 与验收；没有任务板时用 PR 正文的分片表。
- 依赖关系用阻塞表达：被阻塞的片不开工，不靠口头顺序（`docs/workflow/README.md`）。

## 四、分支与 worktree 隔离
- 任务分支从最新 `main` 切；worker 分支从任务分支切，命名 `task/<任务>/<分片>`。
- worktree 放 `.worktrees/<分片>`；该目录已在 `.gitignore` 忽略（`.gitignore`）。
- 共享大目录 `target`、`platform`、`node_modules`、`.tools` 用 junction / symlink 指回主工作区；`.home` 不共享。
- worker 默认不 push 子分支、不开 PR；需要备份时只推子分支，不触发验收。
- 集成完成后删 worktree 与子分支；不把 worktree 留在仓库里当长期目录（`docs/workflow/multi-agent.md`）。

## 五、串行集成
- Lead 按依赖与风险顺序逐片集成；一次只处理一片，不同时改多个分片。
- 每片流程：核对 diff 范围 → 跑该片验收 → 共享文件同提交更新 → 提交 → push 任务分支 → 更新 PR。
- 冲突由该片 owner 在自己的 worktree 里 rebase 解决；跨 owner 冲突由 Lead 重新分片，不手工混改。
- 全量门禁与 CI 在全部集成后由 Lead 跑一次；worker 分支不单独触发 CI（`node run-tests.js`）。
- 同一文件、共享文件与全局收紧入口由 Lead 串行写；不在多个 worktree 并行跑全局收紧或全量测试（`docs/workflow/README.md`）。

## 六、切换与回退
- 单 / 多 agent 可随时切换；先改 PR 的「工作流」说明，已完成提交保留。
- 协调成本过高、契约反复变化或集成连续冲突时，回退单 agent 或重排分片，不硬并行。
- 回退时 Lead 接管未完成分片，清理对应 worktree 与子分支；已集成提交不回退，除非用户确认。
- 切换不改变任务分支与 PR；不因切换另开新 PR（`docs/workflow/README.md`）。

## 七、收口
- 所有 worker 停止；worktree 与子分支清理；`git status` 干净，最终 diff 只含本任务。
- 按 [README.md](README.md) 与 [single-agent.md](single-agent.md) 的收口清单跑全量门禁、差异自查与文档同步。
- 额外检查：分片全覆盖、共享文件只由 Lead 改、PR 分片记录与最终 diff 一致。
- 最终 PR 正文、验收证据与缺口销账由 Lead 完成；用户确认后 Ready / 合并（`AGENTS.md`）。

## 检查
- 分片范围 → PR 分片表或任务板 `writeScopes`（`docs/workflow/multi-agent.md`）。
- 调查证据 → worker 报告中的路径、命令与输出（`docs/workflow/README.md`）。
- 集成完整性 → 任务分支提交序列、每片子门禁与最终全量门禁（`node run-tests.js`）。
- worktree 与分支清理 → `git worktree list`、`git branch`（`docs/workflow/multi-agent.md`）。

## 路由
- 流程总入口：[README.md](README.md)
- 单 agent 工作流：[single-agent.md](single-agent.md)
- 文档审计：[docs-audit.md](docs-audit.md)
- 全仓常驻规则：[../../AGENTS.md](../../AGENTS.md)
- CI 报告细节：[../testing/execution-ci.md](../testing/execution-ci.md)
- PR 模板：[../../.github/pull_request_template.md](../../.github/pull_request_template.md)
