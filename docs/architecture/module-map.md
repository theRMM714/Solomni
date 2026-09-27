# 模块地图

> 本文是**模块地图的唯一权威**：`kernel/`、`core/`、`adapters/`、`presentation/` 各文件职责一览。
> 分层规则与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md)，呈现层入站契约见 [contracts.md](contracts.md)。

## 一、`kernel/`（机制型内核）

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 内核入口：只声明模块，不放逻辑 |
| `types.rs` | 跨业务共享的**事实类型**：只放没有领域逻辑的（`SessionId`） |
| `log.rs` | `Log` 端口（三级）与测试用的 `NoopLog`；文件/时间戳/目录机制在适配层 |
| `path.rs` | 路径的**对外书写形式**（一律 `/`）：跨平台机制，与任何业务无关 |
| `host.rs` | `HostProbe`：宿主能力探测（路径存在性 / PATH 可执行文件 / 本机虚拟化）——**只读事实** |
| `chain.rs` | 任务链的**纯数据 + 纯图算法**（节点、依赖、阶段、就绪与验收判定）。被三个能力共享（`collab` 驱动 / `session` 的线格式携带 / 呈现层渲染），自己零出边（见 [task-chain.md](task-chain.md)） |
| `jobs.rs` | 生成中作业的**取消表**：核心登记，呈现层只能说「停哪个会话」；「停止」不排队、不碰核心状态，所以生成期间立刻生效 |

## 二、`core/`（**应用服务**，无 IO；**正在被搬空**——端口与全部业务能力已落位 `capabilities/`，业务状态也随批次 17 起回到各能力，这里只剩会话中心、生成驱动与跨能力用例）

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 核心层入口与 `Core` 应用服务：会话中心（会话表、命令队列、运行态）、生成驱动、运行包报告、**回档编排**（`rewind` / `rewind_children` / `rebuild_session`——纯算术在 `capabilities/session/domain/rewind.rs`）。登记处与提示词册**只按能力面用**：`registry: Box<dyn Registry>`（`registry()` 读、`registry_mut()` 写）、`prompt: Arc<dyn Prompt>`（按名字取段）与 `systools: Arc<dyn Tools>`（按角色发放工具面），**看不见它们的字段、也不替它们落盘/存册子/存表** |
| `api.rs` | **入站契约 + 入站词汇**：核心自己的两个接口（`SessionOps` 会话中心 / `CoreOps` 核心用例）+ `LogOps`（埋点门面）+ **`Ops` 的组装**（其余接口归各能力，见 §四）+ `CoreHandle`（核心自有线程、命令/事件；各能力接口的**队列代理**也在这里实现）+ **用例词汇与视图**（`WorkMode` / `WorkSpec` / `AgentInstance` / `WorkOpened` / `SessionEdit` / `CollabStep` / `SessionView` / `RuntimeReport` / `FilesView` 等，批次 15 收口从 `mod.rs` 搬来）+ `EventBus`；单 agent 与协作长步骤的生成都在**工作线程**上跑（队列只占"取/交"两步） |

## 三、`adapters/`（机制，实现**内核**端口）

> **本目录正在搬空**：能力私有的实现已按批次 16 归各能力 `detail/`；这里只剩实现 **kernel** 端口的部分（`Log` / `HostProbe`）与入口层用的 `root`。

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 适配层出口（只剩下面这些**内核端口**的实现与入口层用的机制） |
| `host_probe.rs` | `HostProbe`：宿主能力探测（路径存在性、PATH 上的可执行文件、本机虚拟化能力）——**只读事实**，不执行、不安装、不写。`find_exe` 是**可执行文件查找的唯一一份**（`has_exe` 与自检报告共用） |
| `root.rs` | 产品根规范化：传入的根 → 干净的绝对路径（词法拼接优先；取不到当前目录才 canonicalize，并剥掉 Windows 的扩展长度前缀） |
| `log.rs` | `Log`：`logs/` 下按时间戳一份文件 |

## 四、`capabilities/`（业务能力）

| 文件 | 职责 |
| --- | --- |
| `prompt/api.rs` | **入站能力面**：`Prompt` 能力面（按名字取段 `text` / `render` + 两块共享记录 `tools()` / `refs()`）与它要用的词汇（`Segment` / `ToolTexts` / `RefsPrompts` / `RefRoots` / `rewrite` / `Vars`） |
| `prompt/service.rs` | **本能力的状态与用例**：把 `api::Prompt` 挂在 `domain` 的 `Prompts` 上（册子纯数据、无端口），并给组合根一个装载入口 `load()`。**册子只被这里（`Arc<dyn Prompt>` 的持有者）持有**（批次 17） |
| `prompt/ports.rs` | `PromptSource`：提示词册加载（从 `core/ports.rs` 随能力搬出） |
| `prompt/domain/prompt.rs` | 册子的内存形态与 `{{key}}` 渲染（从 `core/prompt.rs` 搬来，纯逻辑） |
| `prompt/domain/refs.rs` | 用户 `@` 引用改写成真实绝对路径（从 `core/refs.rs` 搬来，纯逻辑） |
| `prompt/detail/yaml_prompts.rs` | `PromptSource`：加载 `prompts/`（**只有文本**） |
| `registry/api.rs` | **入站能力面**：`RegistryOps`（呈现层的队列面）+ `Registry`（能力面）+ 登记处词汇（`Settings` / `Provider` / `ModelEntry` / `AppSettings` / 各视图 / `RosterPick`）的对外名字 + **`Registry` 能力面**（读取 `&self`、写取 `&mut self`：状态住在 core 的执行线程上，靠单线程命令队列互斥，**不额外上锁**） |
| `registry/service.rs` | **本能力的状态与用例**：四份 yaml 的内存形态（`Settings`，私有字段）只由这里写；持 `SettingsStore` / `ChatGateway` / `ModelCatalog` / `Log`；装配只在组合根（批次 17） |
| `registry/ports.rs` | `SettingsStore`：登记处四份 yaml 的持久化（从 `core/ports.rs` 随能力搬出） |
| `registry/domain/providers.rs` | 供应商/模型登记处内存形态与「模型 → 通道」解析（从 `core/providers.rs` 搬来） |
| `registry/domain/agents.rs` | agent 登记处、代拟名单落地与名字校验（从 `core/agents.rs` 搬来） |
| `registry/detail/yaml_settings.rs` | `SettingsStore`：登记处四份 yaml 的读写（见 [REGISTRY_SPEC.md](../../REGISTRY_SPEC.md)） |
| `llm/api.rs` | **入站能力面**：`Chat` / `ChatGateway` / `ModelCatalog` / `EnvelopeRepair` 与协议类型（`Msg` / `Completion` / `Chunk` / `ToolCall` / …）的对外名字 |
| `llm/ports.rs` | 模型通道的端口族与协议类型（从 `core/ports.rs` 随能力搬出）；**通道事实**：`Channel`（摊平的解析结果）、`ToolMode`、`ReplayShape` / `ReplayReport`（批次 15 从 `registry` 移来） |
| `workspace/api.rs` | **入站能力面**：`WorkspaceOps`（清单事实）+ 模块清单 / 运行包库 / 执行档位与计划 / 沙箱寻址的对外名字 |
| `workspace/ports.rs` | `ModuleSource` / `PackageSource` / `Workspace`（从 `core/ports.rs` 随能力搬出） |
| `workspace/domain/module.rs` | `module.yaml` 契约、`Roster`、`runtimes` 校验、agent system 合成、**参数声明形态** `Param` / `ParamType`（从 `core/module.rs` 搬来；批次 15 从 `tools` 移来） |
| `workspace/domain/packages.rs` | `package.yaml` 契约与包库事实（从 `core/packages.rs` 搬来） |
| `workspace/domain/exec.rs` | 执行档位（`ExecSpec`）与执行计划（`ExecPlan`）派生、虚拟机档诊断与承载判定（从 `core/exec.rs` 搬来） |
| `workspace/domain/workspace.rs` | 工作区与沙箱的纯数据定义、寻址与越界判定（从 `core/workspace.rs` 搬来） |
| `tools/api.rs` | **入站能力面**：`Tools` 能力面（按角色发放工具面 `tool_face` / `role_face` / `allows_module_tools`、总表 `book()`、自检 `problems()`）+ 工具清单 / 参数契约 / 补丁与应用 / 围栏策略的对外名字 |
| `tools/service.rs` | **本能力的状态与用例**：把 `api::Tools` 挂在 `domain` 的 `SystemTools` 上（两张表是纯数据、无端口），并给组合根一个装载入口 `load()`。**两张表只被这里（`Arc<dyn Tools>` 的持有者）持有**（批次 17） |
| `tools/ports.rs` | `SysIo` / `ToolRunner` / `FenceHost` / `SystoolsSource`（后者的真实实现在 `detail/yaml_systools.rs`） |
| `tools/domain/systool.rs` | 内置工具的放行、寻址、**按声明校验参数**、改动前的"读过"证据（`Observations`）、自由格式补丁的原子应用与回执文案 |
| `tools/domain/patch.rs` | 补丁通道的**纯逻辑**：解析自由格式补丁与整行应用 |
| `tools/domain/schema.rs` | 工具参数契约（**声明在文本层**）：解析/校验/两种渲染 |
| `tools/domain/module_tools.rs` | **清单 → 工具面**：`ToolDecl::schema` / `check_tools` / `module_tools` / `module_tool_params`（批次 15 从 `workspace` 移来） |
| `tools/domain/roles.rs` | 系统工具与**角色**表（`systools/` 两张表） |
| `tools/domain/fence.rs` | 一次工具执行的围栏策略（纯数据） |
| `prompt/detail/yaml_prompts.rs` | `PromptSource`：加载 `prompts/`（**只有文本**） |
| `registry/detail/yaml_settings.rs` | `SettingsStore`：登记处四份 yaml 的读写 |
| `session/detail/fs_history.rs` | `HistoryStore`：`meta.yaml` + `transcript.jsonl` |
| `workspace/detail/fs_modules.rs` | `ModuleSource`：扫描 `modules/`（**保留名表由组合根注入**——清单校验归 workspace，名字空间归 tools） |
| `workspace/detail/fs_packages.rs` | `PackageSource`：扫描 `runtimes/` |
| `workspace/detail/fs_workspace.rs` | `Workspace`：`session/<工作名>/` 下的 work 与各 agent 沙箱 |
| `llm/detail/http_chat.rs` | `Chat` / `ChatGateway`：OpenAI 兼容 `/chat/completions`（请求体形状的唯一定义：真实会话与探针共用） |
| `llm/detail/http_probe.rs` | 两条诊断探针（只报事实）：工具调用支持探测、回放形状探测 |
| `llm/detail/repair.rs` | `EnvelopeRepair`：信封修复（默认只做两类可判定的修补） |
| `llm/detail/http_agent.rs` | 出站 HTTP 代理构建（TLS 后端选择与超时的单点） |
| `llm/detail/endpoint.rs` | 端点补全/回落规则与进程内端点记忆（纯逻辑） |
| `llm/detail/model_catalog.rs` | `ModelCatalog`：OpenAI 兼容 `GET /models` |
| `llm/detail/fake_chat.rs` | 演示/测试通道：`FakeChat`（脚本回放）+ `DemoGateway`（无模型时回落） |
| `tools/detail/confine/` | 守门进程与平台围栏后端（一个平台一个文件） |
| `tools/detail/proc_tools.rs` | `ToolRunner`：守门进程拉起、stdin 送参、超时杀树、输出截断 |
| `tools/detail/sys_io.rs` | `SysIo`：内置工具的读写机制（UTF-8 解码、非法字节 `lossy` 标注） |
| `tools/detail/yaml_systools.rs` | `systools/tools.yaml` + `roles.yaml` → `SystemTools`（**工具总表与角色表是工具侧的事实**，不是提示词） |
| `session/api.rs` | **入站能力面**：`HistoryOps`（落盘会话的列表 / 打开 / 删除）+ `SessionParams` / `AgentSession` / `TurnRun` / 行与事件词汇 / 历史视图的对外名字 |
| `session/ports.rs` | `HistoryStore`：会话历史的持久化（从 `core/ports.rs` 随能力搬出；`core/ports.rs` 随之消失） |
| `session/domain/session.rs` | 会话状态与簿记 + 工具面 `MemberTools` / `ModuleTools`（从 `core/session.rs` 搬来）+ `env_block`（批次 15 从 `tools` 移来） |
| `session/domain/history.rs` | 会话元信息与历史视图的内存形态（从 `core/history.rs` 搬来） |
| `session/domain/events.rs` | 呈现侧契约：`SessionEvent` 与介入请求的词汇、转录行 `LineView`（从 `core/events.rs` 搬来） |
| `session/detail/fs_history.rs` | `HistoryStore`：`meta.yaml` + `transcript.jsonl` |
| `session/domain/rewind.rs` | 回档的**纯行 / 事件算术**：`turn_of_line` / `last_line_within` / `truncate_events` / `cut_before_line` / `align_keep` / `line_reply_of` / `find_line_id`（从 `core/mod.rs` 搬来；**编排留在门面**） |
| `collab/api.rs` | **入站能力面**：`CollabSession` / 讨论与执行引擎 / 回合与验收词汇 / 协作状态派生的对外名字 |
| `collab/domain/collab.rs` | 协作会话状态机与讨论泵（从 `core/collab.rs` 搬来） |
| `collab/domain/engine.rs` | 讨论/执行/验收引擎 + **唯一的轮循环** `converse_with` + **唯一的请求装配点** `assemble` + 回合驱动 + 行构造（从 `core/engine.rs` 搬来） |
| `collab/domain/collab_state.rs` | 「转录即状态」的协作状态派生（从 `core/collab_state.rs` 搬来，纯函数、可回放） |
| `llm/domain/envelope.rs` | 发言信封解析（从 `core/envelope.rs` 搬来，纯逻辑）：`ToolInvoke.body` = 信封之后的正文；判定**未闭合 / 裸控制字符 / 语法错 / 字段不合法**四类；未闭合带上 EOF 状态 |

## 五、`cli/` + `web/`（前端，交付机制）

**两者完全分开**：各渠道一个顶层目录，**互不依赖**，也**没有共享层**（唯一共享的是各能力的 `api`）。
它们**不是业务能力**——没有自己的状态、没有独立不变式；只做三件事：传输、路由、**纯渲染**。

| 文件 | 职责 |
| --- | --- |
| `cli/mod.rs` | 终端转录中心：argv → 入站能力面 → 渲染事件流。`split_names` 与空登记处的引导文案是**它自己的传输侧**的事 |
| `web/mod.rs` | Web 转录中心：tiny_http + 长轮询增量推送（只绑 `127.0.0.1`），分发由路由目录驱动 |
| `web/routes.rs` | **HTTP 入站契约的唯一定义**：`ROUTES` 目录 + 匹配器（[contracts.md](contracts.md) 的表与它机器比对） |
| `web/assets/` | 浏览器端：`app.js` / `md.js` / `style.css` / `index.html`，以及 `*.smoke.cjs` 冒烟 |

## 六、入口层（`main.rs` + `diagnostics/` + `guard/`）

**它依赖所有人，任何人都不许依赖它**（门禁判定）。三个入口各司其职，互不借用：

| 文件 | 职责 |
| --- | --- |
| `main.rs` | **组合根**：`new` 出所有适配器 → 注入 `Core` → 交给某一前端；只做装配与分发，无业务 |
| `diagnostics/mod.rs` | **机器可读探针**（测试与 CI 的接口，不是用户功能）：`--doctor` / `--https-check` / `--print-routes` / `--print-fence-env` / `--fence-verify`。多数**恒退出 0**——判定归调用方 |
| `guard/mod.rs` | **围栏守门进程**：`--fence-run`（装围栏 → 跑模块命令 → 以工具退出码收场）/ `--fence-clean`。它是**第二个程序入口**，跑的是模块作者写的命令 |
