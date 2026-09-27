# 模块地图

> 本文是**模块地图的唯一权威**：`kernel/`、`capabilities/`（含 `conductor/`）、`entry/`、`presentation/` 各文件职责一览。
>
> **路径约定**：每张表的第一格一律是**仓库根相对路径**（`src/…`），由 T0 结构审查机器比对：
> ① 表里出现的每个路径都必须真实存在；② `src/` 下每个 `.rs` 都要有一行
> （`src/tests/**` 归测试分区、只声明模块与重导出的目录入口不必逐行列出）。
> 分层规则与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md)，呈现层入站契约见 [contracts.md](contracts.md)。

## 一、`kernel/`（机制型业务）

> **无领域语义、无领域状态**的机制；形状与别的业务一致（`api` / `ports` / `domain` / `detail`），
> 但它在依赖图的最底层：**不认识任何能力**。
> `Log` / `HostProbe` 是**全项目共享**的机制端口（R12 的例外）：谁都可以持有它们。

| 文件 | 职责 |
| --- | --- |
| `src/kernel/mod.rs` | 机制入口：只声明模块，不放逻辑 |
| `src/kernel/api.rs` | **对外面**：共享事实与纯机制——`SessionId` / `Tier` / `DEFAULT_LLM_TIMEOUT_SECS`（跨业务共享且无领域逻辑）、`slash`（路径对外书写形式）、`JobRegistry`（生成中作业的**取消表**：谁都能登记与取消，「停止」不排队、不碰核心状态，所以生成期间立刻生效） |
| `src/kernel/ports.rs` | **机制端口**：`Log`（三级）与 `NoopLog`、`HostProbe`（宿主能力探测：路径存在性 / PATH 可执行文件 / 本机虚拟化——**只读事实**） |
| `src/kernel/domain/types.rs` | 跨业务共享的**事实类型**：只放没有领域逻辑的 |
| `src/kernel/domain/path.rs` | 路径的**对外书写形式**（一律 `/`）：跨平台机制，与任何业务无关 |
| `src/kernel/domain/jobs.rs` | 取消表的实现（原子标志 + 表；无领域语义） |
| `src/kernel/detail/file_log.rs` | `Log`：`logs/` 下按时间戳一份文件 |
| `src/kernel/detail/host_probe.rs` | `HostProbe`：本机实现。`find_exe` 是**可执行文件查找的唯一一份**（`has_exe` 与自检报告共用） |

## 二、`capabilities/conductor/`（**协调业务**：跨参与方的状态与编排）

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：会话在世表、命令队列、运行态与跨会话编排。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conductor/mod.rs` | 能力入口：只声明模块，不放逻辑 |
| `src/capabilities/conductor/api/mod.rs` | **入站契约**：协调业务自己的两个接口（`SessionOps` 会话中心 / `ConductorOps` 协调用例）+ `LogOps`（埋点门面）+ **`Ops` 的组装**（其余接口归各能力，见 §四）+ `ConductorHandle`（自持线程、命令/事件；各能力接口的**队列代理**也在这里实现）+ **用例词汇与视图**（`WorkMode` / `WorkSpec` / `AgentInstance` / `WorkOpened` / `SessionEdit` / `CollabStep` / `SessionView` / `RuntimeReport` / `FilesView` 等）+ `EventBus`；单 agent 与协作长步骤的生成都在**工作线程**上跑（队列只占"取/交"两步） |
| `src/capabilities/conductor/api/handle.rs` | `ConductorHandle`：把核心搬到它自己的执行线程（命令队列与事件台）、单 agent 与协作的生成驱动、`call` 的取/交两步（队列代理的实现面在 `proxy.rs`） |
| `src/capabilities/conductor/api/proxy.rs` | **队列代理**：`ConductorHandle` 对 `SessionOps` / `ConductorOps` / `RegistryOps` / `HistoryOps` / `WorkspaceOps` / `LogOps` 的实现（只转发，不做业务判断） |
| `src/capabilities/conductor/service/mod.rs` | `Conductor` 本体：会话在世表（会话表、命令队列、运行态）、会话取放与生成驱动、`scan`、测试访问器与基础状态 |
| `src/capabilities/conductor/service/work.rs` | **工作与会话配置**：运行包报告、会话配置读改、建工作与上传、文件视图、会话视图（在世会话 × 历史） |
| `src/capabilities/conductor/service/turn.rs` | **回合收发与协作动作**：拟名单分发、单 agent 的话与准备、协作推进入口、会话生命周期（`ensure_session`） |
| `src/capabilities/conductor/service/flow.rs` | **协作流水线推进**：链推进、节点派发与子会话生成、协作步与恢复（取对象 / 推进 / 放回） |
| `src/capabilities/conductor/service/rewind.rs` | **跨会话回档编排**：`rewind` / `rewind_children` / `rebuild_session`（纯行算术在 session 的 `domain/rewind.rs`） |
| `src/capabilities/conductor/service/env.rs` | **装配材料**：成员通道、沙箱清单、工具环境、预算与角色工具面、单 agent 会话对象 |
| `src/capabilities/conductor/service/history.rs` | **历史与落盘**：历史列出 / 打开 / 删除、事件收编与增量落盘手柄（`Persister`） |

族文件都在同一份 `impl Conductor` 的语义下：与 `mod.rs` 同在 `service` 模块（子模块看得见私有字段），方法取 `pub(crate)`。

## 三、`entry/`（**入口层共用机制**）

> 只有程序入口能用它（组合根 / `diagnostics` / `guard`）；**任何能力都不许依赖它**（门禁判定）。

| 文件 | 职责 |
| --- | --- |
| `src/entry/mod.rs` | 入口层机制出口 |
| `src/entry/root.rs` | 产品根规范化：传入的根 → 干净的绝对路径（词法拼接优先；取不到当前目录才 canonicalize，并剥掉 Windows 的扩展长度前缀） |

## 四、`capabilities/`（业务能力）

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/prompt/api.rs` | **入站能力面**：`Prompt` 能力面（按名字取段 `text` / `render` + 两块共享记录 `tools()` / `refs()`）与它要用的词汇（`Segment` / `ToolTexts` / `RefsPrompts` / `RefRoots` / `rewrite` / `Vars`） |
| `src/capabilities/prompt/service.rs` | **本能力的状态与用例**：把 `api::Prompt` 挂在 `domain` 的 `Prompts` 上（册子纯数据、无端口），并给组合根一个装载入口 `load()`。**册子只被这里（`Arc<dyn Prompt>` 的持有者）持有** |
| `src/capabilities/prompt/ports.rs` | `PromptSource`：提示词册加载 |
| `src/capabilities/prompt/domain/prompt.rs` | 册子的内存形态与 `{{key}}` 渲染（纯逻辑） |
| `src/capabilities/prompt/domain/refs.rs` | 用户 `@` 引用改写成真实绝对路径（纯逻辑） |
| `src/capabilities/prompt/detail/yaml_prompts.rs` | `PromptSource`：加载 `prompts/`（**只有文本**） |
| `src/capabilities/registry/api.rs` | **入站能力面**：`RegistryOps`（呈现层的队列面）+ `Registry`（能力面）+ 登记处词汇（`Settings` / `Provider` / `ModelEntry` / `AppSettings` / 各视图 / `RosterPick`）的对外名字 + **`Registry` 能力面**（读取 `&self`、写取 `&mut self`：状态住在协调业务的执行线程上，靠单线程命令队列互斥，**不额外上锁**） |
| `src/capabilities/registry/service.rs` | **本能力的状态与用例**：四份 yaml 的内存形态（`Settings`，私有字段）只由这里写；持 `SettingsStore` / `ChatGateway` / `ModelCatalog` / `Log`；装配只在组合根 |
| `src/capabilities/registry/ports.rs` | `SettingsStore`：登记处四份 yaml 的持久化 |
| `src/capabilities/registry/domain/providers.rs` | 供应商/模型登记处内存形态与「模型 → 通道」解析 |
| `src/capabilities/registry/domain/agents.rs` | agent 登记处、代拟名单落地与名字校验 |
| `src/capabilities/registry/detail/yaml_settings.rs` | `SettingsStore`：登记处四份 yaml 的读写（见 [REGISTRY_SPEC.md](../../REGISTRY_SPEC.md)） |
| `src/capabilities/llm/api.rs` | **入站能力面**：**通道与协议词汇**（`Chat` / `BoxedChat` / `Msg` / `Completion` / `Chunk` / `ToolCall` / `ToolDecl` / `Channel` / `ToolMode` / `LlmOpts` / `CompleteOpts` / 探测结论 → 它们是**对外契约**，`Chat` 的调用方是别的能力所以我在这里）+ **`Llm` 用例面**（造通道 / 探测 / 发现 / 修信封）。**出站端口不进 api**（R12）。原行余下：协议类型（`Msg` / `Completion` / `Chunk` / `ToolCall` / …）的对外名字 |
| `src/capabilities/llm/service.rs` | **本能力的用例与端口持有者**：持 `ChatGateway` / `ModelCatalog` / `EnvelopeRepair`（**唯一持有者**，R12），实现 `api::Llm` |
| `src/capabilities/llm/ports.rs` | **出站端口**（只有 `service.rs` 持有）：`ChatGateway` / `ModelCatalog` / `EnvelopeRepair`。**通道事实**：`Channel`（摊平的解析结果）、`ToolMode`、`ReplayShape` / `ReplayReport` |
| `src/capabilities/workspace/api.rs` | **入站能力面**：`WorkspaceOps`（呈现层的清单事实）+ **`Workspace` 用例面**（roster / library / runtimes_dir / prepare / write_work / work_has / files / roots）+ 模块清单 / 运行包库 / 执行档位与计划 / 沙箱寻址的对外名字 |
| `src/capabilities/workspace/service.rs` | **本能力的用例与端口持有者**：持 `ModuleSource` / `PackageSource` / `Workdirs`（**唯一持有者**，R12），实现 `api::Workspace` |
| `src/capabilities/workspace/ports.rs` | **出站端口**（只有 `service.rs` 持有）：`ModuleSource` / `PackageSource` / `Workdirs`（目录布局；能力面叫 `Workspace`） |
| `src/capabilities/workspace/domain/module.rs` | `module.yaml` 契约、`Roster`、`runtimes` 校验、agent system 合成、**参数声明形态** `Param` / `ParamType` |
| `src/capabilities/workspace/domain/packages.rs` | `package.yaml` 契约与包库事实 |
| `src/capabilities/workspace/domain/exec.rs` | 执行档位（`ExecSpec`）与执行计划（`ExecPlan`）派生、虚拟机档诊断与承载判定 |
| `src/capabilities/workspace/domain/workspace.rs` | 工作区与沙箱的纯数据定义、寻址与越界判定 |
| `src/capabilities/slate/api.rs` | **入站能力面**：`slate` 的用例面（`propose`）+ DTO（`Mode` / `Pick` / `Proposal` / `Parties` / `Request`）。**没有端口、也没有状态**——名单是在飞的值（归提出方），参与方事实由调用方传入（`collab` 只持登记处快照） |

| `src/capabilities/slate/service.rs` | 用例实现：请核心按需求报名单（`slate` 工具的 `picks` 载荷）→ 按登记处与工作区核验 → 按模式收束。两个调用方共用它：`conductor`（一次性推荐，经 `ConductorOps::suggest_models`）与 `collab`（代拟，待用户确认） |

| `src/capabilities/slate/domain/proposal.rs` | 名单的**纯收束规则**：协作 = 原样；单 agent = 最多一条（多条并成一个临时 agent：模块去重、模型取首项、理由合并） |

| `src/capabilities/taskchain/api.rs` | **入站能力面**（**纯领域业务**：有不变式、无端口、无 `service`）：任务链的事实与派生（`TaskChain` / `TaskNode` / `NodeStatus` / `Acceptance` + 阶段 / 就绪 / 验收判定）。三个消费者都经它：`collab` 驱动、`session` 线格式携带、呈现层渲染 |
| `src/capabilities/taskchain/domain/chain.rs` | 任务链的**纯数据 + 纯图算法**（节点、依赖、阶段派生、就绪、验收判定与装配错误上报）——不做 IO、不碰会话（见 [task-chain.md](task-chain.md)） |
| `src/capabilities/tools/api.rs` | **入站能力面**：`Tools` 能力面（按角色发放工具面 `tool_face` / `role_face` / `allows_module_tools`、总表 `book()` / 自检 `problems()`）+ **`ToolExec` 执行面**（`run_module` / `run_builtin` / `release_fence`）；`ToolOutcome` 从 ports 归到这里。原行余下：`Tools` 能力面（按角色发放工具面 `tool_face` / `role_face` / `allows_module_tools`、总表 `book()`、自检 `problems()`）+ 工具清单 / 参数契约 / 补丁与应用 / 围栏策略的对外名字 |
| `src/capabilities/tools/service/mod.rs` | **本能力的状态、用例与端口持有者**：持两张表（`SystemTools`）与三个出站端口（`ToolRunner` / `SysIo` / `FenceHost`，**唯一持有者**，R12），实现 `api::Tools` 与 `api::ToolExec`；组合根用 `ToolsService::new(加载器, 三个端口)` 装配 |
| `src/capabilities/tools/ports.rs` | **出站端口**（只有 `service.rs` 持有，R12）：`SysIo` / `ToolRunner` / `FenceHost` / `SystoolsSource`（后者的真实实现在 `detail/yaml_systools.rs`） |
| `src/capabilities/tools/service/systool.rs` | 内置工具的**执行编排**：驱动 `SysIo` 读写盘（`read` / `write` / `edit` / `patch` / `list` / `search`）。纯规则在 `domain/systool.rs`——所以只有这里引 `ports` |
| `src/capabilities/tools/domain/systool.rs` | 内置工具的**纯规则**：放行、寻址、按声明校验参数、改动前的"读过"证据（`Observations`）、回执与失败文案、`ToolOutcome`（不引 `ports`） |
| `src/capabilities/tools/domain/patch.rs` | 补丁通道的**纯逻辑**：解析自由格式补丁与整行应用 |
| `src/capabilities/tools/domain/schema.rs` | 工具参数契约（**声明在文本层**）：解析/校验/两种渲染 |
| `src/capabilities/tools/domain/module_tools.rs` | **清单 → 工具面**：`ToolDecl::schema` / `check_tools` / `module_tools` / `module_tool_params` |
| `src/capabilities/tools/domain/roles.rs` | 系统工具与**角色**表（`systools/` 两张表） |
| `src/capabilities/tools/domain/fence.rs` | 一次工具执行的围栏策略（纯数据） |
| `src/capabilities/session/detail/fs_history.rs` | `HistoryStore`：`meta.yaml` + `transcript.jsonl` |
| `src/capabilities/workspace/detail/fs_modules.rs` | `ModuleSource`：扫描 `modules/`（**保留名表由组合根注入**——清单校验归 workspace，名字空间归 tools） |
| `src/capabilities/workspace/detail/fs_packages.rs` | `PackageSource`：扫描 `runtimes/` |
| `src/capabilities/workspace/detail/fs_workspace.rs` | `Workdirs`：`session/<工作名>/` 下的 work 与各 agent 沙箱 |
| `src/capabilities/llm/detail/http_chat.rs` | `Chat` / `ChatGateway`：OpenAI 兼容 `/chat/completions`（请求体形状的唯一定义：真实会话与探针共用） |
| `src/capabilities/llm/detail/http_probe.rs` | 两条诊断探针（只报事实）：工具调用支持探测、回放形状探测 |
| `src/capabilities/llm/detail/repair.rs` | `EnvelopeRepair`：信封修复（默认只做两类可判定的修补） |
| `src/capabilities/llm/detail/http_agent.rs` | 出站 HTTP 代理构建（TLS 后端选择与超时的单点） |
| `src/capabilities/llm/detail/endpoint.rs` | 端点补全/回落规则与进程内端点记忆（纯逻辑） |
| `src/capabilities/llm/detail/model_catalog.rs` | `ModelCatalog`：OpenAI 兼容 `GET /models` |
| `src/capabilities/llm/detail/fake_chat.rs` | 演示/测试通道：`FakeChat`（脚本回放）+ `DemoGateway`（无模型时回落） |
| `src/capabilities/tools/detail/confine/mod.rs` | 守门进程与平台围栏的**调度面**：选平台后端、共享的目录与命令分隔符推导（`windows_program_separators` / `interpreter_dirs` 的共用实现） |
| `src/capabilities/tools/detail/confine/linux.rs` | **进程级围栏**（landlock）：装规则再 exec，规则随进程消失——不需要记账或撤销 |
| `src/capabilities/tools/detail/confine/macos.rs` | **进程级围栏**（seatbelt）：同上，规则是生成出来的一段 profile 文本 |
| `src/capabilities/tools/detail/confine/other.rs` | 其他平台后端：如实降级（只有进程树与超时，不假装有文件系统围栏） |
| `src/capabilities/tools/detail/confine/windows/` | **AppContainer 围栏**（Windows 独有）：`mod.rs` 平台入口 · `acl.rs` 改 DACL 与 ACE / SID 解析 · `record.rs` 授权记录与撤销清扫 · `container.rs` 容器内启动 · `tests.rs` ACL 语义的内联测试（改动在盘上持久，所以要记账） |
| `src/capabilities/tools/detail/confine/windows/mod.rs` | **平台入口**：`capability` / `self_check` / `prepare_fence` / `verify` / `run_fenced`（const 与 FFI 声明也在这里） |
| `src/capabilities/tools/detail/confine/windows/acl.rs` | 安全描述符操作：逐路径改 DACL、解析 ACE 与 SID、展开泛型掩码 |
| `src/capabilities/tools/detail/confine/windows/record.rs` | **授权记录与撤销**：ACL 改动在盘上持久，所以要落记录、按记录撤销并清扫残留档案 |
| `src/capabilities/tools/detail/confine/windows/container.rs` | AppContainer 档案与 SID、UTF-16 转换、system shell、kill-on-close job 对象、容器内拉进程 |
| `src/capabilities/tools/detail/confine/windows/tests.rs` | ACL 语义的内联测试（`#[cfg(test)] mod tests`）：授权 / 撤销 / 清扫 |
| `src/capabilities/tools/detail/proc_tools.rs` | `ToolRunner`：守门进程拉起、stdin 送参、超时杀树、输出截断 |
| `src/capabilities/tools/detail/sys_io.rs` | `SysIo`：内置工具的读写机制（UTF-8 解码、非法字节 `lossy` 标注） |
| `src/capabilities/tools/detail/yaml_systools.rs` | `systools/tools.yaml` + `roles.yaml` → `SystemTools`（**工具总表与角色表是工具侧的事实**，不是提示词） |
| `src/capabilities/session/api.rs` | **入站能力面**：`HistoryOps`（呈现层的队列面：列表 / 打开 / 删除）+ **`History` 直连面**（别的能力用：造会话 / 写元信息 / 追流水 / 列出 / 读回 / 删除；与端口一一对应，价值在 R12 的唯一持有者）+ `SessionParams` / `AgentSession` / `TurnRun` / 行与事件词汇 / 历史视图的对外名字 |
| `src/capabilities/session/service.rs` | **本能力的用例与端口持有者**：持 `HistoryStore`（**唯一持有者**，R12），实现 `api::History`（造会话 / 追流水 / 读元信息 / 删会话）。呈现层的列表/打开/删除仍走 `api::HistoryOps`（队列代理实现）；**核心操作回路** `core_operation`（声明角色工具面 → 跑一次模型 → 从工具调用参数取载荷 → 只读核实回路；取消与分片的包装只有这一处）与 `reply_msgs`（一次模型回复 → 发给模型的消息，**唯一构造函数**）也在这里 |
| `src/capabilities/session/ports.rs` | **出站端口**（只有 `service.rs` 持有，R12）：`HistoryStore`——会话历史的持久化（meta + append-only 流水） |
| `src/capabilities/session/domain/session.rs` | 会话状态与簿记（`AgentSession`、行/回合/回复簿记）+ `SessionParams` / `env_block` + 分片与命名（`stream_piece` / `keep_whole_replies` / `unique_work_name`） |
| `src/capabilities/session/domain/tools.rs` | **成员工具面**：`ModuleTools` / `MemberTools`（含 `next_reply` / `tools_block`）+ `tool_table`（模块清单 → 按模块索引的工具面） |
| `src/capabilities/session/domain/history.rs` | 会话元信息与历史视图的内存形态 |
| `src/capabilities/session/domain/events.rs` | 呈现侧契约：`SessionEvent` 与介入请求的词汇、转录行 `LineView` |
| `src/capabilities/session/domain/rewind.rs` | 回档的**纯行 / 事件算术**：`turn_of_line` / `last_line_within` / `truncate_events` / `cut_before_line` / `align_keep` / `line_reply_of` / `find_line_id` / `max_reply`（转录里用过的最大回复号，重建时续号）（**编排在协调业务**） |
| `src/capabilities/collab/api.rs` | **入站能力面**：`CollabSession` / 讨论与执行引擎 / 回合与验收词汇 / 协作状态派生的对外名字 |
| `src/capabilities/collab/service/collab.rs` | **协作会话状态机**：会话对象与它的状态（名单、方案、任务链、挂起）、需求提交、审查关卡与节点验收、待裁决判定 |
| `src/capabilities/collab/service/pump.rs` | **讨论泵**：推进一步（`pump_with`）并装配成员（`assemble_members`）；行外送与增量（`emit_new_lines` / `push_delta` / `derive_pending` / `review_event`） |
| `src/capabilities/collab/service/turn_io.rs` | **回合收发**：成员回复回填（`feed_with`）、取出待问的一步（`take_ask`）、回答与裁决（`answer` / `decide`）、核心核实工具面与通道参数 |
| `src/capabilities/collab/service/slate.rs` | **代拟与确认、恢复与收尾**：`draft_slate` / `confirm_slate` / `begin`、断点续跑（`resume`）与终结判定（`is_done`） |
| `src/capabilities/collab/service/discussion.rs` | **讨论状态机**：建组 → 轮转发言 → 表态判定 → 全员同意后交整理（`Member` / `Discussion` / `MemberTurn` / `Adv` / `AfterTurn`） |
| `src/capabilities/collab/service/synthesis.rs` | **整理与审查**：核心整理讨论出方案与任务链（`synthesize`，走 plan 工具）+ 总验收清单与返工判定（`Execution` / `CheckItem`） |
| `src/capabilities/collab/service/round.rs` | **轮循环与行构造**：一次成员回复的完整翻译（`converse_with`）、请求装配（`assemble`）、行构造（`build_round_lines`）、轮与动词词汇 |
| `src/capabilities/collab/service/tool_loop.rs` | 成员回合里的**工具循环**：声明面 → 放行判定 → 并发调度 → 执行（内置 / 模块外部）→ 回填（`run_one` / `run_batch` / `dispatch_external` / `tool_decls`…） |
| `src/capabilities/collab/service/driver.rs` | **回合驱动**（自由函数，会话当参数）：`say` / `dispatch_task` / `discussion_turn` / `continue_reply` / `compact_turn` / `maybe_compact` / `run_rounds` / `rounds_events` / `run`。为什么不是 `impl AgentSession`：给别人的类型写 impl 是另一种互相引入（R1） |
| `src/capabilities/collab/domain/collab_state.rs` | 「转录即状态」的协作状态派生（纯函数、可回放） |
| `src/capabilities/llm/domain/envelope.rs` | 发言信封解析（纯逻辑）：`ToolInvoke.body` = 信封之后的正文；判定**未闭合 / 裸控制字符 / 语法错 / 字段不合法**四类；未闭合带上 EOF 状态 |
| `src/capabilities/llm/domain/malformed.rs` | 信封不合法的**回执文案装配**：按判定出的类别给出修法。模板住在 `prompt`（`ToolTexts`），选择与拼装在 `llm`——放 `prompt` 会成环（见 ARCHITECTURE.md §一） |

## 五、`cli/` + `web/`（前端，交付机制）

**两者完全分开**：各渠道一个顶层目录，**互不依赖**，也**没有共享层**（唯一共享的是各能力的 `api`）。
它们**不是业务能力**——没有自己的状态、没有独立不变式；只做三件事：传输、路由、**纯渲染**。

| 文件 | 职责 |
| --- | --- |
| `src/cli/mod.rs` | 终端转录中心：argv → 入站能力面 → 渲染事件流。`split_names` 与空登记处的引导文案是**它自己的传输侧**的事 |
| `src/web/mod.rs` | Web 转录中心：tiny_http + 长轮询增量推送（只绑 `127.0.0.1`），分发由路由目录驱动 |
| `src/web/routes.rs` | **HTTP 入站契约的唯一定义**：`ROUTES` 目录 + 匹配器（[contracts.md](contracts.md) 的表与它机器比对） |
| `src/web/assets/` | 浏览器端：`app.js` / `md.js` / `style.css` / `index.html`，以及 `*.smoke.cjs` 冒烟 |

## 六、入口层（`main.rs` + `diagnostics/` + `guard/`）

**它依赖所有人，任何人都不许依赖它**（门禁判定）。三个入口各司其职，互不借用：

| 文件 | 职责 |
| --- | --- |
| `src/main.rs` | **组合根**：`new` 出所有适配器 → 注入 `Core` → 交给某一前端；只做装配与分发，无业务 |
| `src/diagnostics/mod.rs` | **机器可读探针**（测试与 CI 的接口，不是用户功能）：`--doctor` / `--https-check` / `--print-routes` / `--print-fence-env` / `--fence-verify`。多数**恒退出 0**——判定归调用方 |
| `src/guard/mod.rs` | **围栏守门进程**：`--fence-run`（装围栏 → 跑模块命令 → 以工具退出码收场）/ `--fence-clean`。它是**第二个程序入口**，跑的是模块作者写的命令 |
