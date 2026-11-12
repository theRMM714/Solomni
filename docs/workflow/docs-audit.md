# 文档审计与治理

> 定位：用户启动「文档审计 / 治理」时的专用流程：怎么检查、改什么、按什么验收。
> 效力：规范｜核对：2026-11-04｜依据：`tests/docs-gov.mjs`、`tests/docs-audit.mjs`、`tests/docs-sync.mjs`

## 适用与判据

- 用户明确说「审计文档 / 治理文档 / 清理过时描述」时，按本文执行（`docs/workflow/docs-audit.md`）。
- 只改文档与文档检查脚本；发现代码问题另开任务（`docs/workflow/README.md`）。
- 一次审计是否完成只看「五、验收标准」，不看读了多少文档（`AGENTS.md` 九）。

## 一、触发与输入

- 触发：用户明确启动，或季度 / 里程碑例行（`docs/workflow/README.md`）。
- 输入：当前 `git` 基线、上次审计的 PR、机器候选清单（`tests/docs-audit.mjs`）。
- 先跑机器检查、再看候选；不做全量通读（`tests/docs-audit.mjs`）。

## 二、检查顺序

- 结构基线：`node run-tests.js --docs`，模板、锚点、生成段先过（`run-tests.js`）。
- 候选清单：`node tests/docs-audit.mjs`，取过期 / 状态缺证据 / 重复事实 / 失效锚点 / 孤儿 / 多处同源（`tests/docs-audit.mjs`）。
- 逐条取证：候选里的每条去读代码、配置、命令输出或 CI 报告，不靠语义猜测（`AGENTS.md` 九）。
- 抽样：本轮改动过的文档与例外文档各抽 1–2 份人工读一遍（`docs/conventions.md`）。
- 修订复跑：改完重跑前两步，结论写进 PR 正文（`docs/workflow/README.md`）。

## 三、检查要求

- 事实源唯一：同一事实只留一处权威，其它处只链接（`docs/conventions.md`）。
- 可执行优先：能用命令、测试或生成段验证的，不写自然语言（`tests/docs-sync.mjs`）。
- 散文收敛：段落拆成带锚点的断言；没有锚点的判断只能写「原则 / 建议」（`AGENTS.md` 九）。
- 过时修正：依据变更、状态词无证据、命令 / 路径失效、章节引用失效，四类都算过时（`tests/docs-audit.mjs`）。
- 不做加法：不新增无约束、无行为、无使用价值的文档（`docs/conventions.md`）。
- 例外文档：`AGENTS.md`、`README*.md`、`docs/presentation/contracts.md`、`docs/prompt/prompts.md` 只能人工复核，不套模板（`docs/templates/index.json`）。

## 四、修改规则

- 只改文档与文档检查脚本；不改产品代码与 CI 工作流（`docs/templates/index.json`）。
- 同次同步：门户、索引、路由表与被引用的标题一起改（`docs/conventions.md`）。
- 生成段只改标记内内容，并用 `node tests/docs-sync.mjs --write` 重生成（`tests/docs-sync.mjs`）。
- 证据不足：写进缺口账或报告，不编造路径、测试名或结论（`tests/gaps.yaml`）。
- 决策理由不进长期文档，需要保留的放 PR 正文（`AGENTS.md` 四）。

## 五、验收标准

- `node run-tests.js --docs` 输出 `TEST-REPORT-OK`（`run-tests.js`）。
- `node tests/docs-sync.mjs` 输出「需更新 0 份」（`tests/docs-sync.mjs`）。
- 候选清单的过期、状态缺证据、失效锚点、孤儿均为 0；重复 / 同源若保留，逐条在 PR 正文说明（`tests/docs-audit.mjs`）。
- `git status` 干净，diff 只含本次候选相关文档（`docs/workflow/README.md`）。
- PR 正文列出：候选清单、处理结论、未处理原因（`docs/workflow/README.md`）。

## 六、与编程工作流的衔接

- 文档治理按「任务」分类，先方案后实施，流程与闸门沿用 `docs/workflow/README.md`；按单 / 多 agent 选择读对应细则。
- 纯文档改动用 `node run-tests.js --docs` 收口，不跑三平台（`docs/testing/execution-ci.md`）。
- 每阶段一个提交，PR 正文随阶段更新（`docs/workflow/README.md`）。

## 检查

- 结构与锚点 → `tests/docs-gov.mjs`（`node run-tests.js --docs` 调用）；判据实现在 `tests/gates/docs/`（4 文件）与 `tests/gates/structure/`（9 文件）。
- 候选清单可复现 → `tests/docs-audit.mjs`。
- 生成段一致 → `tests/docs-sync.mjs`（6 个目标：平台验收状态 ×4、T0 步骤、成功标记）。

## 路由

- 任务流程与 Git 规则：[README.md](README.md)
- 文档分层、术语与引用：[../conventions.md](../conventions.md)
- 模板与例外：[../templates/index.json](../templates/index.json)
