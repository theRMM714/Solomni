# 提示词册（prompts/）

> 本文是**提示词册结构与键清单的唯一权威**：每份文件里有什么键、每个键干什么。
> 规则（谁写、占位符、缺键报错、什么不进册子）见 [ARCHITECTURE.md](../../ARCHITECTURE.md) 的「五、提示词册」；
> "哪个角色用哪份提示词、拿哪些工具"见 [tools-and-roles.md](../tools/tools-and-roles.md) 的「提示词按角色分配」；
> 模块自己的能力摘要（`brief`）与工具用法（`system`）见 [MODULE_SPEC.md](../../MODULE_SPEC.md)。

册子按**共享 / 角色**两个目录切分；加载经 `PromptSource` 端口（`capabilities/prompt/ports.rs`，实现在 `capabilities/prompt/detail/yaml_prompts.rs`），
`{{key}}` 渲染与"缺键即报错"在 `capabilities/prompt/domain/prompt.rs`（纯逻辑，不读文件）。
键的完整清单就是下面这张表——**本表由 `node run-tests.js` 的 T0 结构审查与盘上的册子双向比对**：加了键没改表、表里的键已不存在，都是硬失败。

## 一、结构与键

| 文件 | 键 | 用途 |
| --- | --- | --- |
| `shared/mechanisms.yaml` | `session_kinds` / `mechanisms[].session` / `mechanisms[].roles` / `mechanisms[].text` | **机制说明按（会话使用类型 × 适用角色）分发**：一个会话的身份块把**同时**匹配它类型与角色的条目按声明顺序拼起来（`{{mechanism}}`）。`session_kinds` 是类型全表，`roles` 校验落在角色表（`systools/roles.yaml`）——对应关系全在数据里，代码只做匹配（`CoreTexts::mechanisms_for`） |
| `shared/protocol.yaml` | `chat_protocol` | 讨论约定（讨论席与执行席一起注入，可自由演化） |
| `shared/tools.yaml` | `env_hosts` / `env_guests` / `envs[].host\|guest\|text` | **工作环境按环境分发**：本机档按宿主平台（`env_hosts`：linux / windows / macos），虚拟机档按 guest 的 os（`env_guests`）；每条只写 host 或 guest 之一，装载时校验全表覆盖（不静默漏发一份环境）。文本变量：work_name / agent / work_root / sandbox_root / module_roots / path_rules——虚拟机档给的是 **guest 内路径**；路径规矩只有一处（`texts.yaml` 的 `path_rules`，`env_block` 用它渲染后插进每个环境块） |
| | `patch_guide` | 自由格式补丁的写法（每块以 `*** End File` 收尾、SEARCH 要整行一致、一次可多块、整体原子） |
| | `tool_calling_envelope` / `tool_calling_native` | 工具调用约定**两套，互斥**：一个通道只用一套，由通道形态决定注入哪套 |
| `shared/texts.yaml` | `module_tool_params_header` | 模块工具参数段的小标题（静态模块工具或已就绪服务操作声明了 `params` 时出现；两类参数合并成一段，标题只出现一次） |
| | `path_rules` / `tools_this_turn` / `compact_prompt` / `discuss_reminder` | **指令段**（进入模型上下文，但属于"模型被要求做什么"）：路径写法、本回合工具说明块标题、压缩回合指令、讨论轮次里"上一轮没表态"的提醒 |
| `shared/agent.yaml` | `agent.system` / `agent.chara` | agent 身份骨架（人设块 + 模块 `brief` 能力行 + `system` 工具用法 + 该会话**类型 × 角色**的机制说明 + 工作环境 + 调用约定；**工具清单不在这里**，随回合注入）。`system` 的变量：agent / chara_block / modules / mechanism / env / tool_calling；`chara` 是独立人设段（变量 chara，空人设不插入空标题） |
| `roles/discussant.yaml` | `discuss.opener` / `discuss.step` / `discuss.autonomy_note` | 讨论首轮、轮转、小组自裁说明 |
| `roles/planner.yaml` | `synthesize.*` | 整理方案 |
| | `node_review.*` | 节点验收（产出"节点 id — 负责人"表与结论） |
| | `slate.*` | 名单（推荐 / 代拟同一条协议）：`mode_single` / `mode_collab` 是编排形态的两种说法，`system` / `user` 是载荷说明与清单 |
| | `verdict.*` | 判定用户那一句是否明确（明确才开工 / 放行，`collab::judge_clear`） |
| `roles/executor.yaml` | `execute.user` | 执行任务 |
| `roles/orchestrator.yaml` | `review.system` / `review.user` | 总验收与推进 |
| `roles/core_proxy.yaml` | `proxy.system` | 核心代理的身份提示词（在用户授予的任务级授权范围内代用户决定与转达；工具面由角色表按回合注入）。代理会话**不是 agent**：它的身份块渲染这一段（`{{mechanism}}` 取 `mechanisms.proxy`，`{{env}}` / `{{tool_calling}}` 与 agent 身份同一份口径，见 session 的角色身份块 `session::domain::session::role_system`），不是模块能力包 |

> 运行时回执**不在册子**：工具回执与参数/补丁错误在 `tools`，信封不合法的修法在 `llm`，围栏/进程收尾在 `kernel`，模块分派与轮次标注在 `collab`，清单行在 `registry` / `modules` / `tools`，`@` 引用说明在 `prompt/domain/refs.rs`。**路径校验与权限拒绝由 workspace 代码直接返回。**

> `solo`（用户建的单 agent 工作）的工具面**复用 `roles/executor.yaml`**：它不发回报工具，而 `execute.user` 那段
> 只在协作的派发行上渲染——所以没有第二份文件，也没有复述。

## 二、怎么被装载、怎么被取用

- `PromptSource` 端口（`capabilities/prompt/ports.rs`，实现在 `detail/yaml_prompts.rs`）按文件装配成
  `Prompts`：各文件的**顶层键合并**成 `core:` 的内容（键在两份文件里重复 = 装配错误，不静默覆盖）；
- **册子只由提示词能力持有一次**（`capabilities/prompt/service.rs`）：组合根 `prompt::service::load()` 得到
  `Arc<dyn Prompt>`，协调业务持它、协作会话与它**共享同一份**；
- **别的能力不点字段路径**，取用只有一条路：**按名字取一段**——`Prompt::text(Segment::…)`（原文）
  或 `Prompt::render(Segment::…, vars)`（渲染）；名字表是 `capabilities/prompt/domain/prompt.rs` 的 `Segment`——
  **加一段提示词 = 册子加键 + 这里加变体**（缺了编译不过）。运行时回执不由本能力返回。
- 渲染（`{{key}}` 替换、缺键 / 缺变量即报错）在 `capabilities/prompt/domain/prompt.rs`（纯逻辑，不读文件）；
- **工具总表与角色表不在这份册子里**：`systools/tools.yaml`（工具是什么）与 `systools/roles.yaml`（身份有什么）
  由 `capabilities/tools/detail/yaml_systools.rs` 的 `YamlSystools` 装配成 `SystemTools`，**与册子分开注入**——
  挂进册子会让提示词反过来依赖工具，两边成环（见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一）；
- **组装（哪一回合发哪几段）留在各业务**：身份块归 `session`、工具说明归 `tools`、清单文本归 `registry` / `workspace`
  ——prompt 只给"段"，不替它们拼（否则它反过来要认识会话与工具）。
