# 单 agent 工作流

> 定位：一个 agent 串行完成任务：开工、阶段实施、本机回路与收口细则。
> 效力：规范｜核对：2026-11-13｜依据：`docs/workflow/README.md`

## 适用与判据
- 选择单 agent 时按本文执行；共同六步、闸门、分支与 PR 规则见 [README.md](README.md)。
- 一个 agent 承担全部调查、实现、提交、PR 与门禁，不把工作转给未登记的 agent。
- 违反由 PR 检查与 `node run-tests.js` 判定；agent 不自行 Ready / 合并（`AGENTS.md`）。

## 一、开工：分支 + Draft PR
- 顺序固定：

1. 从最新 `main` 切任务分支：
   ```sh
   git fetch origin main
   git switch -c <分支> origin/main
   ```
2. 空提交占位并推送：
   ```sh
   git commit --allow-empty -m "chore: 开工 <任务>"
   git push -u origin <分支>
   ```
3. 立刻创建 Draft PR：
   ```sh
   gh pr create --draft --base main --head <分支>
   ```
4. 正文按 [.github/pull_request_template.md](../../.github/pull_request_template.md) 写全：目标与范围、明确不做、验收标准、实施清单、风险。
5. 开工自检（全部为是才写实现代码）：
   - [ ] 分支从最新 `main` 切出并 push
   - [ ] 空提交已推送
   - [ ] Draft PR 已创建
   - [ ] PR 正文含方案 / 验收 / 实施清单 / 明确不做
- 任一项为否：停下修，不要先写代码（`.github/pull_request_template.md`）。
- 无法创建或访问 PR（无远程、无权限、网络不通）：停下报告，不要先干完再补（`AGENTS.md`）。
- Draft 期不跑 CI：本机按「三、本机开发回路」迭代；需要他平台结论时手动派发（见 [README.md](README.md) 的 Git 与 CI）。Ready 之后要继续做 WIP，可以转回 Draft。
- 决定、证据、CI 结论记录到 PR；PR 是任务进度的唯一落点。

## 二、实施：阶段提交 + PR 更新
1. 阶段性小步提交 = 一个可独立验收的行为 / 契约变化；一个阶段一个提交（用户另有要求时按用户）。
2. 每个阶段：代码 + 测试 + 文档同一提交 → push → 更新 PR 的实施清单、阶段记录、测试结论、决定与风险。
3. 阶段结束时 PR 未更新 = 该阶段没有完成；不要等收口再补 PR 正文。
4. 严格按已批准范围开发，不擅自扩大修改；优先用工具和可观察证据判断。
5. 遇到阻塞或方案需要实质变化：先改 PR 正文或评论说明事实，再询问用户，不硬做。
6. 原方案范围内暴露的实现错误可直接修复；证明原方案错误时，回到方案阶段重新确认。
7. 范围外的问题记进 PR（或缺口账），不混进本次改动。
8. 测试与文档跟代码同一次提交：新行为有新用例，改了行为改对应用例；权威文档改成当前状态，不留「实施中」。
9. 完成修改和适用测试后再提交；提交前检查 `.gitignore` 与 `git status`，不提交无关修改。

## 三、本机开发回路
- 改一层验一层用 `node dev.js test|fmt|clippy`（它用项目内工具链与缓存，见 [execution-ci.md](../testing/execution-ci.md) 的「本机开发回路」）。
- 迭代时按门禁判一次用 `node run-tests.js --skip-e2e`，验收用 `node run-tests.js`。
- 查清单 / 步骤 / 标记用 `node run-tests.js --list`；只跑需要的几项用 `--only=<gate|lane|step>`，任意子集不写报告、不能当验收。
- 纯叙述型文档改动用 `node run-tests.js --docs`（文档审查 + 结构审查，不跑代码 / 三平台 / L4；`docs/presentation/contracts.md` 除外，它走全量）。
- cargo 只经这些入口跑（透传口 `node dev.js cargo -- <参数>`）；要在终端直接敲，先按 `env.js` 注入环境——`CARGO_HOME`、`RUSTUP_HOME`、`TMPDIR` / `TMP` / `TEMP` 与 `PATH` 的清单见 [execution-ci.md](../testing/execution-ci.md) 的「本机开发回路」；不注入会用 PATH 上的系统工具链并把缓存、临时目录写到项目外（`AGENTS.md`）。

## 四、收口
1. 按 `TESTING.md` 运行适用测试和质量检查：代码任务本地 `node run-tests.js` 全绿；纯叙述型文档任务用 `node run-tests.js --docs`，只代表文档审查 + 结构审查通过。失败则继续修复，不把 `env-skip`、`gap` 或 `blocked` 写成通过。
2. 差异自查：`git status` 干净、无无关改动、无机器路径与用户信息、PR 只含本任务。
3. 交用户确认（闸门 2）。
4. 用户确认后缺口销账：`tests/gaps.yaml` 删已补条目，并按规则同步门户与现状描述；检查权威文档，删掉过期、临时和矛盾的说法。
5. 把 PR 正文改成最终状态：做了什么 / 未做 / 风险，并写明验收证据（命令、成功标记、CI 的 `sha`）。
6. 用户确认后，Ready 与合并由用户执行；agent 不自行 Ready、不自行合并。PR 标题即主线提交信息，要能独立说明本次任务主题；合并后删任务分支（`AGENTS.md`）。

## 检查
- 流程闸门 → 用户确认与 Draft PR 正文（`.github/pull_request_template.md`）。
- 开工自检 → 分支、空提交、Draft PR 与 PR 正文（`docs/workflow/README.md`）。
- 本机回路 → `node dev.js test|fmt|clippy` 与 `node run-tests.js`（`docs/testing/execution-ci.md`）。
- 收口门禁 → `node run-tests.js`（纯文档 `node run-tests.js --docs`）。

## 路由
- 流程总入口：[README.md](README.md)
- 多 agent 工作流：[multi-agent.md](multi-agent.md)
- 文档审计：[docs-audit.md](docs-audit.md)
- 全仓常驻规则：[../../AGENTS.md](../../AGENTS.md)
- CI 报告细节：[../testing/execution-ci.md](../testing/execution-ci.md)
- PR 模板：[../../.github/pull_request_template.md](../../.github/pull_request_template.md)
