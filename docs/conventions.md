# 文档与术语约定

> 定位：文档分层、同步、术语与引用格式的唯一细则。
> 效力：规范｜核对：2026-11-25｜依据：`AGENTS.md`

## 适用与判据
- 写或改文档、定术语、写引用时以本文为准（`docs/conventions.md`）。
- 违反由 `node run-tests.js --docs` 的文档审查与结构审查判定。

## 一、两层文档

- **根门户**（`README.md`、`ARCHITECTURE.md` 等）只写「这份文档管什么、要做事时读哪一份」，不复述细则正文；例外是 `AGENTS.md`，它承载始终生效的约束与任务受理流程，本身即权威。
- **细则**在 `docs/<子业务>/`：每个业务子业务与机制单元各一个目录（业务单元本身不另建目录，由 `docs/architecture/layers.md` 的分层表与 `docs/README.md` 索引归纳），目录里的 `README.md` 是唯一入口，`unit-map.md` 是逐文件职责（T0 机器比对）。
- 不属任何单元的横向内容按事项分：测试与质量在 `docs/testing/`，任务受理与开工在 [../AGENTS.md](../AGENTS.md) 的「六、请求受理」，单 / 多 agent 与文档审计细则在 `workflow/` 目录，文档约定与术语在本文，Rust 源码规范在 [architecture/rust-dev.md](architecture/rust-dev.md)。
- 只有任务引入新的长期领域且现有文档无法承载时，才建新专项文档；不为单次任务创建无长期价值的文档。
- 细则之间用相对路径互引；门户用 `docs/<领域>/<文件>.md` 引用细则。同一个事实只有一份权威。

## 二、改文档必须同步

- 下列任一处改动，都要在同一次提交里把相关方一起改掉，不得留下过期引用或两处说法（`AGENTS.md`）：

| 改了什么 | 必须同时检查 |
| --- | --- |
| 细则正文（`docs/**`） | 该领域的门户、[`AGENTS.md`](../AGENTS.md) 文档路由表、其它文档里指向该节的引用 |
| 门户的节标题或结构 | 该领域细则里的回引、[`AGENTS.md`](../AGENTS.md) 文档路由表、[`README.md`](../README.md) 文档表 |
| 新增 / 删除 / 改名文档 | [`AGENTS.md`](../AGENTS.md) 文档路由表、[`README.md`](../README.md) 文档表、所有引用它的文档与代码注释 |
| 代码里被文档机器比对的段落 | 该文档与比对它的测试（路由表 ↔ `src/tests/routes.rs`、提示词键表 ↔ `prompts/**`、`unit-map.md` ↔ 磁盘、根 `*.md` ↔ 文档路由表的表行、`docs/*/README.md` ↔ `docs/README.md` 索引表） |
| `tests/gaps.yaml` 的条目 | [`TESTING.md`](../TESTING.md) 门户与 [`gaps-acceptance.md`](testing/gaps-acceptance.md) 的现状描述 |

- 引用一律用**相对仓库根的路径**（`docs/testing/levels.md`），不写机器路径、不写绝对路径。
- 文档里的当前状态必须与代码一致；未实现的内容写进缺口账，不写成当前能力（`tests/gaps.yaml`）。
- 门户与细则都只保留当前状态，不写变更历史。

## 三、术语

- 同一个概念全仓只用一个词；下表是唯一口径（左列是要用的词）：

| 概念 | 用 | 不用 |
| --- | --- | --- |
| 发言与行动的主体 | **agent** | 智能体、AI、模型（模型是 agent 用到的东西） |
| 能力的载体（一个文件夹 = 提示词 + 工具 + 工作区） | **模块**（方向性叙述里可以说"应用 / app"） | 插件、技能、skill |
| 代码里按边界切分的一块：`layers.yaml` 一行一个（业务单元即 `src/units/[pure/]<单元>/`，`kernel` 是环 0 的单元）；根（入口层）与呈现层是适配器，不入环表 | **单元**（unit） | 模块、组件 |
| 业务边界（`src/units/<单元>/<子业务>/`；独立业务的判据见 `docs/architecture/boundaries.md`） | **子业务**（能力 / capability） | 业务模块、模块 |
| 一次任务里"用谁、怎么组合、谁拍板" | **形态**（single / collab / proxy） | 模式、架构、拓扑 |
| 一个会话类型的组成（id 与显示名、流程序列、是否整块交给核心、没有流程时的静止机制） | **会话类型**（`Profiles`；数据在 `systools/profiles.yaml`；落盘是 `SessionSpec`） | 会话模板、流程模板 |
| 回合身份的来源：一组席位（核心席与成员席各要几员）加上它发哪些工具；`flow: true` 的能当流程串进会话，其余是静止机制 | **机制**（`Mechanism`；声明在 `src/units/session_model/domain/mechanism.rs` 的 `MECHANISMS`） | 角色组、流程模板 |
| 机制里的一个身份组：提示词 + 工具面 + 能不能写 / 能不能发布 + 是否给模块工具 | **席位**（`Seat`；同一文件的 `SEATS`，id 全局唯一） | 角色、角色表 |
| 驱动与具体流程之间的插件契约（推一步 / 让出一步 / 交回 / 处置自己发的关卡） | **流程**（`Flow`；`flow: true` 的机制即流程，id 即机制 id） | 流程引擎、编排器 |
| 一次运行与它的落盘单元 | **会话**（session） | 对话、聊天 |
| 挂在父会话 `children/` 下的会话 | **子会话**（强调"它是一场完整工作"时可写**子工作**） | 子 agent、子任务 |
| 用户投喂与成品的地方 | **共享区**（`work/`；整棵树只有一个） | 工作区、workspace（除非确指 `data/session/<工作名>/` 这个目录） |
| 共享区里按字节管理的文件（文本 / 图片 / 视频 / 二进制） | **通用资产**（必要时简称**资产**） | 文件（需要与“文本文件”区分时）、附件 |
| 那个唯一的程序 | **核心** | 内核、核心进程（`kernel` 是地基库单元，不是核心） |
| 机器可读的事件流水 | **转录**（transcript） | 日志、聊天记录 |
| 由核心实现 / 由模块声明的工具 | **系统工具 / 模块工具** | 外部工具（= 模块工具） |
| 系统工具里由 `tools` 单元直接实现、不经模块或核心动作的那组 | **内置工具**（`read` / `write` / `edit` / `patch` / `delete` / `list` / `grep` / `submit_report`）；其中操作文件的几个也叫**内置文件工具** | — |
| 系统工具里执行外部命令、经 kernel 进程机制（守门进程 / guest）跑的那组 | **执行类系统工具**（当前 `shell`） | 终端、shell 工具 |
| 模块声明的常驻外部服务（MCP / harness 等） | **常驻服务**（单元 `residents`） | 外部服务、后台进程、daemon |
| 模块声明的隐秘信息（值归产品、只落 `data/.home/`） | **隐秘字段**（单元 `secrets`） | 密钥（除非确指通道密钥）、凭据、secret |
| 多 agent 工作流里对用户与最终结果负责的 agent | **Lead**（主控 agent） | 主 agent、协调者 |
| 多 agent 工作流里只负责一个分片的 agent | **worker**（分片 agent） | 子 agent、执行者 |

## 四、引用格式

- 引用一律用**文件 + 标题**（例如 `docs/session/session-model.md` 的「会话参数与对话分开」），不写深层节号（`§三之二` 这类会随节序失效）。
- 引用写「文件 + 标题」；`docs/architecture/` 各细则的顶层节号（「一、…」这类）也可作锚点，不写深层节号。
- 链接按所在文件解析：根门户写 `docs/<...>`，`docs/` 内部写同级或 `../` 相对路径。

## 五、文档模板与效力行

- 每份受治理文档必须符合 `docs/templates/<类型>.md` 的骨架；类型映射与例外在 `docs/templates/index.json`（门户 / 规范 / 参考 / 单元 / 表）。例外仅限 `AGENTS.md`、`README.md`、`README_EN.md`、PR 模板、两处机器契约（`docs/presentation/contracts.md`、`docs/prompt/prompts.md`）与模板目录自身。
- 头部固定两行：`> 定位：…` 与 `> 效力：<索引|规范|契约>｜核对：YYYY-MM-DD｜依据：…`；规范 / 契约必须带核对与依据（校验在 `tests/docs-gov.mjs`）。`核对` 是人工确认日，不是文件 mtime。
- 骨架之外只允许「重复块」：同一个块模板重复 N 次（规则组 / 字段组 / 专题 / 表行），N 由对象数量决定；不得新增模板外的固定章节（唯一来源：`docs/templates/`）。
- 块内只允许列表项、表格、代码块、引用行；每块最多 1 行纯段落引导语且 ≤150 字。强约束句（必须 / 不得 / 禁止 / 只允许 / 只能 / 一律 / 零容忍）必须带锚点；没有锚点的判断只能写成「原则 / 建议」（`AGENTS.md` 九）。
- 模板即契约：改 `docs/templates/**` 必须同一次提交改完受影响文档，否则检查失败。

## 检查
- 文档分层与路由 → 文档审查的 md 链接与根文档路由检查、结构审查的 `unitMap` 与代码注释文档引用（`node run-tests.js`）。
- 文档模板、效力行与锚点 → `docs-gov`（`node run-tests.js --docs` 调用，硬失败）。
- 季度审计候选 → `node tests/docs-audit.mjs`（只报不拦：过期 / 状态缺证据 / 重复事实 / 失效锚点 / 孤儿 / 例外）。
- 生成段（平台验收状态 / T0 步骤 / 成功标记）→ `node tests/docs-sync.mjs`（`--check` 由 `--docs` 调用；来源是代码常量、缺口账与 `--list` 同源模块，目标列表在脚本的 `TARGETS`，改生成段同次改它）。
- 术语与引用 → `node run-tests.js --docs` 的文档审查。

## 路由
- 全仓常驻规则：[../AGENTS.md](../AGENTS.md)
- 文档索引：[README.md](README.md)
- 任务受理与开工：[../AGENTS.md](../AGENTS.md) 的「六、请求受理」
- 单 agent 细则：[workflow/single-agent.md](workflow/single-agent.md)
- 多 agent 细则：[workflow/multi-agent.md](workflow/multi-agent.md)
- 文档审计 / 治理流程：[workflow/docs-audit.md](workflow/docs-audit.md)
- 测试细则索引：[testing/README.md](testing/README.md)
