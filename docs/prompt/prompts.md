# 提示词册（prompts/）

> 本文是**提示词册结构与键清单的唯一权威**：每份文件里有什么键、每个键干什么。
> 规则（谁写、占位符、缺键报错、什么不进册子）见 [ARCHITECTURE.md](../../ARCHITECTURE.md) 的「五、提示词册」；
> "哪个角色用哪份提示词、拿哪些工具"见 [tools-and-roles.md](../tools/tools-and-roles.md) 四；
> 模块自己的职责提示词（`module.yaml` 的 `system`）见 [MODULE_SPEC.md](../../MODULE_SPEC.md)。

**当前状态：已落地。** 册子按**共享 / 角色**两个目录切分；加载经 `PromptSource` 端口（`capabilities/prompt/ports.rs`，实现在 `capabilities/prompt/detail/yaml_prompts.rs`），
`{{key}}` 渲染与"缺键即报错"在 `capabilities/prompt/domain/prompt.rs`（纯逻辑，不读文件）。
键的完整清单就是下面这张表——**本表由 `node run-tests.js` 的 T0 结构审查与盘上的册子双向比对**：加了键没改表、表里的键已不存在，都是硬失败。

## 一、结构与键

| 文件 | 键 | 用途 |
| --- | --- | --- |
| `shared/mechanisms.yaml` | `session_kinds` / `mechanisms[].session` / `mechanisms[].roles` / `mechanisms[].text` | **机制说明按（会话使用类型 × 适用角色）分发**：一个会话的身份块把**同时**匹配它类型与角色的条目按声明顺序拼起来（`{{mechanism}}`）。`session_kinds` 是类型全表，`roles` 校验落在角色表（`systools/roles.yaml`）——对应关系全在数据里，代码只做匹配（`CoreTexts::mechanisms_for`） |
| `shared/protocol.yaml` | `chat_protocol` | 讨论约定（讨论席与执行席一起注入，可自由演化） |
| `shared/tools.yaml` | `env` | **工作环境块**：本 agent 的真实根目录（共享区主副本 / 沙箱工作副本 / 模块目录）、路径规矩与共享区版本化（pull / commit / status）的用法 |
| | `patch_guide` | 自由格式补丁的写法（每块以 `*** End File` 收尾、SEARCH 要整行一致、一次可多块、整体原子） |
| | `tool_calling_envelope` / `tool_calling_native` | 工具调用约定**两套，互斥**：一个通道只用一套，由通道形态决定注入哪套 |
| `shared/texts.yaml` | `no_agents` / `no_model` / `no_module_dirs` / `no_module_tools` / `no_module_tool_params` | 空态说法 |
| | `module_tool_params_header` | 模块工具参数段的小标题（模块在 `module.yaml` 里声明了 `params` 时出现） |
| | `refs.foreign_sandbox` / `refs.collab_sandbox` | `@` 引用越权与协作场景的如实说明（定义在 `shared/texts.yaml`） |
| | `tool_texts.*` | **运行时回执**：路径校验、参数不符（说事实 + 回发工具签名）、内置工具回执与行区间/截断/编码标注、edit 的找不到（含"只差空白"提示）与多处命中、patch 的解析失败与"第几块为什么、整体没写"、write 的"没读过/读后又被改/只读到一部分"三种拒绝、外部工具分派的三类失败、**信封不合法四类**与"已修复后执行"/"输出被长度截断"的如实标注、给模型看的清单骨架、追加在回复行末尾的 `（已停止）` / `（本段被输出长度截断）` |
| `shared/agent.yaml` | `agent.system` | agent 职责提示词骨架（模块 `system` 合成 + 该会话**类型 × 角色**的机制说明 + 工作环境 + 调用约定；**工具清单不在这里**，随回合注入） |
| `roles/discussant.yaml` | `discuss.opener` / `discuss.step` / `discuss.autonomy_note` | 讨论首轮、轮转、小组自裁说明 |
| `roles/planner.yaml` | `synthesize.*` | 整理方案 |
| | `node_review.*` | 节点验收（产出"节点 id — 负责人"表与结论） |
| | `slate.*` | 名单（推荐 / 代拟同一条协议）：`mode_single` / `mode_collab` 是编排形态的两种说法，`system` / `user` 是载荷说明与清单 |
| | `verdict.*` | 判定用户那一句是否明确（明确才开工 / 放行，`collab::judge_clear`） |
| `roles/executor.yaml` | `execute.user` | 执行任务 |
| `roles/orchestrator.yaml` | `review.system` / `review.user` | 总验收与推进 |
| `roles/core_proxy.yaml` | `proxy.system` | 核心代理的身份提示词（在用户授予的任务级授权范围内代用户决定与转达；工具面由角色表按回合注入）。代理会话**不是 agent**：它的身份块渲染这一段（`{{mechanism}}` 取 `mechanisms.proxy`，`{{env}}` / `{{tool_calling}}` 与 agent 身份同一份口径，见 `workspace::api::role_system`），不是模块能力包 |

> `solo`（用户建的单 agent 工作）的工具面**复用 `roles/executor.yaml`**：它不发回报工具，而 `execute.user` 那段
> 只在协作的派发行上渲染——所以没有第二份文件，也没有复述。

## 二、怎么被装载、怎么被取用

- `PromptSource` 端口（`capabilities/prompt/ports.rs`，实现在 `detail/yaml_prompts.rs`）按文件装配成
  `Prompts`：各文件的**顶层键合并**成 `core:` 的内容（键在两份文件里重复 = 装配错误，不静默覆盖）；
- **册子只由提示词能力持有一次**（`capabilities/prompt/service.rs`）：组合根 `prompt::service::load()` 得到
  `Arc<dyn Prompt>`，协调业务持它、协作会话与它**共享同一份**；
- **别的能力不点字段路径**，取用只有两条路：
  1. **按名字取一段**：`Prompt::text(Segment::…)`（原文）或 `Prompt::render(Segment::…, vars)`（渲染）；
     名字表是 `capabilities/prompt/domain/prompt.rs` 的 `Segment`——**加一段提示词 = 册子加键 + 这里加变体**
     （缺了编译不过）；
  2. **拿走两块共享记录**：`Prompt::tools()`（`tool_texts`，~100 条模型侧文案）与 `Prompt::refs()`，
     它们是 `Arc`：沙箱、工具环境、会话一律共享同一份，**不逐处深拷贝**；
- 渲染（`{{key}}` 替换、缺键 / 缺变量即报错）在 `capabilities/prompt/domain/prompt.rs`（纯逻辑，不读文件）；
- **工具总表与角色表不在这份册子里**：`systools/tools.yaml`（工具是什么）与 `systools/roles.yaml`（身份有什么）
  由 `capabilities/tools/detail/yaml_systools.rs` 的 `YamlSystools` 装配成 `SystemTools`，**与册子分开注入**——
  挂进册子会让提示词反过来依赖工具，两边成环（见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一）；
- **组装（哪一回合发哪几段）留在各业务**：身份块归 `session`、工具说明归 `tools`、清单文本归 `registry` / `workspace`
  ——prompt 只给"段"，不替它们拼（否则它反过来要认识会话与工具）。
