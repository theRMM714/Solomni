# 多 agent 工作流

> 定位：多个 agent 并行调查或实施：Lead 开工、派发、逐片集成与收口细则。
> 效力：规范｜核对：2026-11-13｜依据：`docs/workflow/README.md`

## 适用与判据

- 选择多 agent 时按本文执行；共同约定与开工步骤（分支、Draft PR、闸门、阶段提交）见 [README.md](README.md)。
- 咨询 / 审查仍只调查、不建分支；实施仍走同一套门禁，多 agent 不改变请求分类与验收。
- 一个任务只有一个 Lead、一个任务分支、一个 Draft PR；worker 只对分片负责。
- 调查型并行不建分支与 worktree；实施型并行按下面的分片、worktree 与逐片集成执行。

## 一、分工与两条路径

- Lead：对用户单点负责；定方案、冻结跨分片契约、拆分片、维护任务板、管共享文件、集成、跑最终门禁、维护 PR。
- worker：只改自己的分片写范围，跑分片验收，向 Lead 报告提交、文件、测试与例外。
- 共享文件：基线 / 棘轮、门禁脚本、登记处、文档门户与路由、PR 正文、跨分片契约文件；由 Lead 独占。
- 两条路径：
  - 调查型：只读取证，不写文件、不建 worktree 与子分支；Lead 列问题清单与每个 worker 的范围。
  - 实施型：分片写范围不相交，Lead 预建 worktree 与子分支，worker 在 worktree 内开发。

## 二、开工：分片、契约与 worktree

- 按 [README.md](README.md) 的「三、开工：分支 + Draft PR」建任务分支 `task/<简述>`、空提交与 Draft PR。
- PR 实施清单写分片表：分片、owner、写范围、子分支、依赖；没有任务板时以这张表为准。
- 冻结跨分片契约（API、端口、数据模型、错误文案、盘上格式）；worker 改契约前先报 Lead。
- Lead 预建子分支与 worktree，一个分片一个：

  ```sh
  git worktree add .worktrees/<分片> -b task/<任务>/<分片> task/<任务>
  ```

- 有依赖的分片等依赖集成后再建 worktree；依赖在任务板记 `blocked_by`，不靠口头顺序。
- 共享目录用链接指回主工作区：`target`、`platform`、`node_modules`、`.tools`（Windows 用 `mklink /J`，macOS / Linux 用 `ln -s`）；`.home` 不共享。
- 分片测试与全量门禁串行：共享 `target` 同时跑会抢锁、写同一份报告。

## 三、派发：派发单与任务板

- 每张派发单写：分片、子分支、worktree、写范围、验收命令、依赖、交付物、报告格式。
- 有任务板时登记分片、`write_scopes`、`blocked_by`、验收命令；没有任务板时用 PR 的分片表。
- 一个文件只有一个 owner；跨分片共享文件由 Lead 改，worker 发现要改时停下报告。
- worker 默认不 push 子分支、不开 PR、不触发 CI；需要备份时只推子分支。

## 四、worker：worktree 内开发与汇报

- 只在 `.worktrees/<分片>` 内工作；开工前确认当前分支是 `task/<任务>/<分片>`。
- 小步提交，每个提交对应一个可验收变化；代码、测试与文档放在同一次提交。
- 本机回路：`node dev.js test|fmt|clippy`；只跑分片验收用 `node run-tests.js --only=<gate|lane|step>`。
- 提交前 rebase 到任务分支，保持可 fast-forward 集成：

  ```sh
  git rebase task/<任务>
  ```

- 不 merge、不改任务分支、不动共享文件；汇报后分片冻结，后续改动先报 Lead。
- 汇报格式：

  ```text
  分片：A
  子分支：task/<任务>/a
  HEAD：<sha>
  改动文件：...
  验收：<命令与结果>
  例外：无 / 具体项
  已 rebase 到：task/<任务>@<sha>
  ```

## 五、逐片集成与验收

- 主工作区切到任务分支，确认干净后再集成一片：

  ```sh
  git switch task/<任务>
  git status --short
  git log --oneline task/<任务>..task/<任务>/<分片>
  git diff --stat task/<任务>...task/<任务>/<分片>
  ```

- 用 squash 把子分支压成一个阶段提交：

  ```sh
  git merge --squash task/<任务>/<分片>
  ```

- 先跑分片验收，通过再提交；失败用 `git reset --hard HEAD` 清场，退回 worker 修复。
- 冲突不手工混改：把子分支退回 owner，在自己的 worktree 里 rebase 任务分支后重新汇报。
- 提交并 push 任务分支：

  ```sh
  git add -A
  git commit -m "<类型>(<范围>): <分片摘要>"
  git push
  ```

- PR 阶段记录逐片追加：分片、子分支、worker HEAD、squash 提交、验收命令与结果、例外。
- 清理该片：

  ```sh
  git worktree remove .worktrees/<分片>
  git branch -D task/<任务>/<分片>
  ```

- 依赖片集成后再建下一个 worktree；同一时间只集成一片。

## 六、收口：清理、全量门禁与交付

- 所有 worker 停止；`git worktree list` 无残留，`git branch` 只剩任务分支。
- `git status` 干净，最终 diff 只含本任务；PR 分片记录与最终 diff 一致。
- 跑全量门禁 `node run-tests.js`（纯文档 `node run-tests.js --docs`）；失败继续修，不把 `env-skip` / `gap` 写成通过。
- 差异自查：无无关改动、无机器路径与用户信息、PR 只含本任务。
- PR 正文改成最终状态：做了什么 / 未做 / 风险，附验收证据（命令、成功标记、CI 的 `sha`）。
- 交用户确认（闸门 2）；用户确认后 Ready 与合并由用户执行，合并后删任务分支。

## 七、切换与回退

- 单 / 多 agent 可随时切换；先改 PR 的「工作流」说明，已完成提交保留。
- 多 agent → 单 agent：Lead 接管未完成分片，清理对应 worktree 与子分支；已集成提交不回退。
- 协调成本过高、契约反复变化或集成连续冲突：回退单 agent 或重排分片，不硬并行。
- 单 agent → 多 agent：按本文「二、开工」补分片表与 worktree；任务分支与 PR 不换。
- 回退已 push 的集成提交用 `git revert <sha>`，不重写任务分支历史；子分支按 PR 记录的原 HEAD 恢复。

## 八、完整示例：task/export-csv

- 分片：A 核心导出（`src/capabilities/export/`）；B CLI 接入，依赖 A；C 文档，依赖 A。
- Lead 按 [README.md](README.md) 的开工步骤建 `task/export-csv` 分支与 Draft PR，然后建 A 的 worktree 并派发：

  ```sh
  git worktree add .worktrees/a -b task/export-csv/a task/export-csv
  ```

- worker 在 `.worktrees/a` 完成后汇报；Lead 集成 A：

  ```sh
  git switch task/export-csv
  git merge --squash task/export-csv/a
  node run-tests.js --only=<A 分片验收>
  git add -A && git commit -m "feat(导出): 核心导出能力"
  git push
  git worktree remove .worktrees/a
  git branch -D task/export-csv/a
  ```

- A 集成后再建 B、C 的 worktree，重复派发与逐片集成：

  ```sh
  git worktree add .worktrees/b -b task/export-csv/b task/export-csv
  git worktree add .worktrees/c -b task/export-csv/c task/export-csv
  ```

- PR 阶段记录样例：

  ```text
  - 分片 A：worker HEAD <sha> → squash <sha>；验收 <命令> 通过；例外：无
  - 分片 B：worker HEAD <sha> → squash <sha>；验收 <命令> 通过；例外：无
  - 分片 C：worker HEAD <sha> → squash <sha>；验收 <命令> 通过；例外：无
  ```

- 全部集成后跑 `node run-tests.js`，更新 PR 终态，交用户确认后 Ready 与合并。

## 检查

- 分片范围与依赖 → PR 分片表或任务板 `write_scopes` / `blocked_by`。
- 工作交接 → 派发单与 worker 汇报（HEAD、验收命令与结果、例外）。
- 集成完整性 → 任务分支提交序列、逐片验收与最终全量门禁（`node run-tests.js`）。
- 清理 → `git worktree list`、`git branch`。

## 路由

- 流程总入口：[README.md](README.md)
- 单 agent 工作流：[single-agent.md](single-agent.md)
- 文档审计：[docs-audit.md](docs-audit.md)
- 全仓常驻规则：[../../AGENTS.md](../../AGENTS.md)
- CI 报告细节：[../testing/execution-ci.md](../testing/execution-ci.md)
- PR 模板：[../../.github/pull_request_template.md](../../.github/pull_request_template.md)
