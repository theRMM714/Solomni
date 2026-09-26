# 提示词册（prompts/）

> 本文是**提示词册结构与键清单的唯一权威**：每份文件里有什么键、每个键干什么。
> 规则（谁写、占位符、缺键报错、什么不进册子）见 [ARCHITECTURE.md](../../ARCHITECTURE.md) 的「五、提示词册」；
> "哪个角色用哪份提示词、拿哪些工具"见 [tools-and-roles.md](tools-and-roles.md) 四；
> 模块自己的职责提示词（`module.yaml` 的 `system`）见 [MODULE_SPEC.md](../../MODULE_SPEC.md)。

**当前状态：已落地。** 册子按**共享 / 角色**两个目录切分；加载经 `PromptSource` 端口（`adapters/yaml_prompts.rs`），
`{{key}}` 渲染与"缺键即报错"在 `core/prompt.rs`（纯逻辑，不读文件）。键的完整清单就是下面这张表。

## 一、结构与键

| 文件 | 键 | 用途 |
| --- | --- | --- |
| `shared/protocol.yaml` | `mechanism` / `chat_protocol` | 工具调用机制的说明 + 讨论约定（两者一起注入，可自由演化） |
| | `refs.foreign_sandbox` / `refs.collab_sandbox` | `@` 引用越权与协作场景的如实说明 |
| `shared/tools.yaml` | `env` | **工作环境块**：本 agent 的真实根目录（共享区 / 沙箱 / 模块目录）与路径规矩 |
| | `patch_guide` | 自由格式补丁的写法（每块以 `*** End File` 收尾、SEARCH 要整行一致、一次可多块、整体原子） |
| | `tool_calling_envelope` / `tool_calling_native` | 工具调用约定**两套，互斥**：一个通道只用一套，由通道形态决定注入哪套 |
| | `builtin_tools` | 内置工具的**参数契约**：模型侧说明与调用校验的唯一来源（不写进代码） |
| `shared/texts.yaml` | `no_agents` / `no_model` / `no_module_dirs` / `no_module_tools` / `no_module_tool_params` | 空态说法 |
| | `module_tool_params_header` | 模块工具参数段的小标题（模块在 `module.yaml` 里声明了 `params` 时出现） |
| | `tool_texts.*` | **运行时回执**：路径校验、参数不符（说事实 + 回发工具签名）、内置工具回执与行区间/截断/编码标注、edit 的找不到（含"只差空白"提示）与多处命中、patch 的解析失败与"第几块为什么、整体没写"、write 的"没读过/读后又被改/只读到一部分"三种拒绝、外部工具分派的三类失败、**信封不合法四类**与"已修复后执行"/"输出被长度截断"的如实标注、给模型看的清单骨架、追加在回复行末尾的 `（已停止）` / `（本段被输出长度截断）` |
| `shared/agent.yaml` | `agent.system` | agent 职责提示词骨架（模块 `system` 合成 + 工作环境 + 调用约定；**工具清单不在这里**，随回合注入） |
| `roles/discussant.yaml` | `discuss.opener` / `discuss.step` / `discuss.autonomy_note` | 讨论首轮、轮转、小组自裁说明 |
| `roles/planner.yaml` | `synthesize.*` | 整理方案 |
| | `node_review.*` | 节点验收（产出"节点 id — 负责人"表与结论） |
| | `slate.*` / `suggest_models.*` | 代拟名单 / 模型推荐（单 agent、协作两种说法） |
| | `verdict.*` | 判定用户那一句是否明确（明确才开工 / 放行，`collab::judge_clear`） |
| `roles/executor.yaml` | `execute.user` | 执行任务 |
| `roles/orchestrator.yaml` | `review.system` / `review.user` | 总验收与推进 |

## 二、怎么被装载

- `PromptSource` 端口（`adapters/yaml_prompts.rs`）按文件装配成 `Prompts`（`core/prompt.rs` 的结构体，字段与键同名）；
- `core/prompt.rs` 只做 `{{key}}` 渲染与"缺键/缺变量即报错"（纯逻辑）；
- 文案的注入方式与端口一致：随环境对象传入（沙箱 / 工具环境 / 引用改写器），不让纯逻辑自己去读文件。
