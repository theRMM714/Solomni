# 模块地图

> 本文是**模块地图的唯一权威**：`kernel/`、`capabilities/`（含 `conductor/`）、`entry/`、`presentation/` 各文件职责一览。
> 分层规则与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md)，呈现层入站契约见 [contracts.md](contracts.md)。

## 一、`kernel/`（机制型业务）

> **无领域语义、无领域状态**的机制；形状与别的业务一致（`api` / `ports` / `domain` / `detail`），
> 但它在依赖图的最底层：**不认识任何能力**。
> `Log` / `HostProbe` 是**全项目共享**的机制端口（R12 的例外）：谁都可以持有它们。

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 机制入口：只声明模块，不放逻辑 |
| `api.rs` | **对外面**：共享事实与纯机制——`SessionId` / `Tier` / `DEFAULT_LLM_TIMEOUT_SECS`（跨业务共享且无领域逻辑）、`slash`（路径对外书写形式）、`JobRegistry`（生成中作业的**取消表**：谁都能登记与取消，「停止」不排队、不碰核心状态，所以生成期间立刻生效） |
| `ports.rs` | **机制端口**：`Log`（三级）与 `NoopLog`、`HostProbe`（宿主能力探测：路径存在性 / PATH 可执行文件 / 本机虚拟化——**只读事实**） |
| `domain/types.rs` | 跨业务共享的**事实类型**：只放没有领域逻辑的 |
| `domain/path.rs` | 路径的**对外书写形式**（一律 `/`）：跨平台机制，与任何业务无关 |
| `domain/jobs.rs` | 取消表的实现（原子标志 + 表；无领域语义） |
| `detail/file_log.rs` | `Log`：`logs/` 下按时间戳一份文件 |
| `detail/host_probe.rs` | `HostProbe`：本机实现。`find_exe` 是**可执行文件查找的唯一一份**（`has_exe` 与自检报告共用） |

## 二、`capabilities/conductor/`（**协调业务**：跨参与方的状态与编排）

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：会话在世表、命令队列、运行态与跨会话编排。

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 能力入口：只声明模块，不放逻辑 |
| `api.rs` | **入站契约**：协调业务自己的两个接口（`SessionOps` 会话中心 / `CoreOps` 协调用例）+ `LogOps`（埋点门面）+ **`Ops` 的组装**（其余接口归各能力，见 §四）+ `ConductorHandle`（自持线程、命令/事件；各能力接口的**队列代理**也在这里实现）+ **用例词汇与视图**（`WorkMode` / `WorkSpec` / `AgentInstance` / `WorkOpened` / `SessionEdit` / `CollabStep` / `SessionView` / `RuntimeReport` / `FilesView` 等）+ `EventBus`；单 agent 与协作长步骤的生成都在**工作线程**上跑（队列只占"取/交"两步） |
| `service.rs` | `Conductor`（协调业务的状态与用例）：会话在世表（会话表、命令队列、运行态）、生成驱动、运行包报告、**跨会话回档编排**（`rewind` / `rewind_children` / `rebuild_session`——纯行/事件算术在 `capabilities/session/domain/rewind.rs`）+ `Persister`（增量落盘）。登记处、会话历史、提示词册、工具面**只按能力面用**（`Box<dyn Registry>` / `Arc<dyn History>` / `Arc<dyn Prompt>` / `Arc<dyn Tools>` …），看不见它们的字段、也不替它们落盘 |

## 三、`entry/`（**入口层共用机制**）

> 只有程序入口能用它（组合根 / `diagnostics` / `guard`）；**任何能力都不许依赖它**（门禁判定）。

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 入口层机制出口 |
| `root.rs` | 产品根规范化：传入的根 → 干净的绝对路径（词法拼接优先；取不到当前目录才 canonicalize，并剥掉 Windows 的扩展长度前缀） |

## 四、`capabilities/`（业务能力）

| 文件 | 职责 |
| --- | --- |
| `prompt/api.rs` | **入站能力面**：`Prompt` 能力面（按名字取段 `text` / `render` + 两块共享记录 `tools()` / `refs()`）与它要用的词汇（`Segment` / `ToolTexts` / `RefsPrompts` / `RefRoots` / `rewrite` / `Vars`） |
| `prompt/service.rs` | **本能力的状态与用例**：把 `api::Prompt` 挂在 `domain` 的 `Prompts` 上（册子纯数据、无端口），并给组合根一个装载入口 `load()`。**册子只被这里（`Arc<dyn Prompt>` 的持有者）持有**（批次 17） |
| `prompt/ports.rs` | `PromptSource`：提示词册加载 |
| `prompt/domain/prompt.rs` | 册子的内存形态与 `{{key}}` 渲染（纯逻辑） |
| `prompt/domain/refs.rs` | 用户 `@` 引用改写成真实绝对路径（纯逻辑） |
| `prompt/detail/yaml_prompts.rs` | `PromptSource`：加载 `prompts/`（**只有文本**） |
| `registry/api.rs` | **入站能力面**：`RegistryOps`（呈现层的队列面）+ `Registry`（能力面）+ 登记处词汇（`Settings` / `Provider` / `ModelEntry` / `AppSettings` / 各视图 / `RosterPick`）的对外名字 + **`Registry` 能力面**（读取 `&self`、写取 `&mut self`：状态住在协调业务的执行线程上，靠单线程命令队列互斥，**不额外上锁**） |
| `registry/service.rs` | **本能力的状态与用例**：四份 yaml 的内存形态（`Settings`，私有字段）只由这里写；持 `SettingsStore` / `ChatGateway` / `ModelCatalog` / `Log`；装配只在组合根（批次 17） |
| `registry/ports.rs` | `SettingsStore`：登记处四份 yaml 的持久化 |
| `registry/domain/providers.rs` | 供应商/模型登记处内存形态与「模型 → 通道」解析 |
| `registry/domain/agents.rs` | agent 登记处、代拟名单落地与名字校验 |
| `registry/detail/yaml_settings.rs` | `SettingsStore`：登记处四份 yaml 的读写（见 [REGISTRY_SPEC.md](../../REGISTRY_SPEC.md)） |
| `llm/api.rs` | **入站能力面**：**通道与协议词汇**（`Chat` / `BoxedChat` / `Msg` / `Completion` / `Chunk` / `ToolCall` / `ToolDecl` / `Channel` / `ToolMode` / `LlmOpts` / `CompleteOpts` / 探测结论 → 它们是**对外契约**，`Chat` 的调用方是别的能力所以我在这里）+ **`Llm` 用例面**（造通道 / 探测 / 发现 / 修信封）。**出站端口不进 api**（R12）。原行余下：协议类型（`Msg` / `Completion` / `Chunk` / `ToolCall` / …）的对外名字 |
| `llm/service.rs` | **本能力的用例与端口持有者**：持 `ChatGateway` / `ModelCatalog` / `EnvelopeRepair`（**唯一持有者**，R12），实现 `api::Llm`（批次 20b） |
| `llm/ports.rs` | **出站端口**（只有 `service.rs` 持有）：`ChatGateway` / `ModelCatalog` / `EnvelopeRepair`。协议词汇已随批次 20b 归 `api`；**通道事实**：`Channel`（摊平的解析结果）、`ToolMode`、`ReplayShape` / `ReplayReport`（批次 15 从 `registry` 移来） |
| `workspace/api.rs` | **入站能力面**：`WorkspaceOps`（呈现层的清单事实）+ **`Workspace` 用例面**（roster / library / runtimes_dir / prepare / write_work / work_has / files / roots）+ 模块清单 / 运行包库 / 执行档位与计划 / 沙箱寻址的对外名字 |
| `workspace/service.rs` | **本能力的用例与端口持有者**：持 `ModuleSource` / `PackageSource` / `Workdirs`（**唯一持有者**，R12），实现 `api::Workspace`（批次 20b） |
| `workspace/ports.rs` | **出站端口**（只有 `service.rs` 持有）：`ModuleSource` / `PackageSource` / `Workdirs`（目录布局；原名 `Workspace`，批次 20b 改名，把 `Workspace` 让给能力面） |
| `workspace/domain/module.rs` | `module.yaml` 契约、`Roster`、`runtimes` 校验、agent system 合成、**参数声明形态** `Param` / `ParamType`（批次 15 从 `tools` 移来） |
| `workspace/domain/packages.rs` | `package.yaml` 契约与包库事实 |
| `workspace/domain/exec.rs` | 执行档位（`ExecSpec`）与执行计划（`ExecPlan`）派生、虚拟机档诊断与承载判定 |
| `workspace/domain/workspace.rs` | 工作区与沙箱的纯数据定义、寻址与越界判定 |
| `taskchain/api.rs` | **入站能力面**（**纯领域业务**：有不变式、无端口、无 `service`）：任务链的事实与派生（`TaskChain` / `TaskNode` / `NodeStatus` / `Acceptance` + 阶段 / 就绪 / 验收判定）。三个消费者都经它：`collab` 驱动、`session` 线格式携带、呈现层渲染 |
| `taskchain/domain/chain.rs` | 任务链的**纯数据 + 纯图算法**（节点、依赖、阶段派生、就绪、验收判定与装配错误上报）——不做 IO、不碰会话（见 [task-chain.md](task-chain.md)） |
| `tools/api.rs` | **入站能力面**：`Tools` 能力面（按角色发放工具面 `tool_face` / `role_face` / `allows_module_tools`、总表 `book()` / 自检 `problems()`）+ **`ToolExec` 执行面**（`run_module` / `run_builtin` / `release_fence`）；`ToolOutcome` 从 ports 归到这里（批次 20b）。原行余下：`Tools` 能力面（按角色发放工具面 `tool_face` / `role_face` / `allows_module_tools`、总表 `book()`、自检 `problems()`）+ 工具清单 / 参数契约 / 补丁与应用 / 围栏策略的对外名字 |
| `tools/service.rs` | **本能力的状态、用例与端口持有者**：持两张表（`SystemTools`）与三个出站端口（`ToolRunner` / `SysIo` / `FenceHost`，**唯一持有者**，R12），实现 `api::Tools` 与 `api::ToolExec`；组合根用 `ToolsService::new(加载器, 三个端口)` 装配（批次 17 + 20b） |
| `tools/ports.rs` | **出站端口**（只有 `service.rs` 持有，R12）：`SysIo` / `ToolRunner` / `FenceHost` / `SystoolsSource`（后者的真实实现在 `detail/yaml_systools.rs`） |
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
| `workspace/detail/fs_workspace.rs` | `Workdirs`：`session/<工作名>/` 下的 work 与各 agent 沙箱 |
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
| `session/api.rs` | **入站能力面**：`HistoryOps`（呈现层的队列面：列表 / 打开 / 删除）+ **`History` 直连面**（别的能力用：造会话 / 写元信息 / 追流水 / 列出 / 读回 / 删除；与端口一一对应，价值在 R12 的唯一持有者）+ `SessionParams` / `AgentSession` / `TurnRun` / 行与事件词汇 / 历史视图的对外名字 |
| `session/service.rs` | **本能力的用例与端口持有者**：持 `HistoryStore`（**唯一持有者**，R12），实现 `api::History`（造会话 / 追流水 / 读元信息 / 删会话）。呈现层的列表/打开/删除仍走 `api::HistoryOps`（队列代理实现）；**核心操作回路** `core_operation`（声明角色工具面 → 跑一次模型 → 从工具调用参数取载荷 → 只读核实回路；取消与分片的包装只有这一处）与 `reply_msgs`（一次模型回复 → 发给模型的消息，**唯一构造函数**）也在这里 |
| `session/ports.rs` | **出站端口**（只有 `service.rs` 持有，R12）：`HistoryStore`——会话历史的持久化（meta + append-only 流水） |
| `session/domain/session.rs` | 会话状态与簿记 + 工具面 `MemberTools` / `ModuleTools`+ `env_block`（批次 15 从 `tools` 移来） |
| `session/domain/history.rs` | 会话元信息与历史视图的内存形态 |
| `session/domain/events.rs` | 呈现侧契约：`SessionEvent` 与介入请求的词汇、转录行 `LineView` |
| `session/detail/fs_history.rs` | `HistoryStore`：`meta.yaml` + `transcript.jsonl` |
| `session/domain/rewind.rs` | 回档的**纯行 / 事件算术**：`turn_of_line` / `last_line_within` / `truncate_events` / `cut_before_line` / `align_keep` / `line_reply_of` / `find_line_id`（**编排在协调业务**） |
| `collab/api.rs` | **入站能力面**：`CollabSession` / 讨论与执行引擎 / 回合与验收词汇 / 协作状态派生的对外名字 |
| `collab/domain/collab.rs` | 协作会话状态机与讨论泵 |
| `collab/domain/engine.rs` | 讨论/执行/验收引擎 + **唯一的轮循环** `converse_with` + **唯一的请求装配点** `assemble` + 回合驱动 + 行构造 |
| `collab/domain/collab_state.rs` | 「转录即状态」的协作状态派生（纯函数、可回放） |
| `llm/domain/envelope.rs` | 发言信封解析（纯逻辑）：`ToolInvoke.body` = 信封之后的正文；判定**未闭合 / 裸控制字符 / 语法错 / 字段不合法**四类；未闭合带上 EOF 状态 |

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
