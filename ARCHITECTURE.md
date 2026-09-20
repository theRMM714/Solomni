# 架构与开发规则（ARCHITECTURE）

> 改代码前必读。本文是**分层的唯一权威**：谁依赖谁、端口画在哪、机制放哪一层、日志与提示词册怎么用、落盘契约长什么样。
> 理念见 [PHILOSOPHY.md](PHILOSOPHY.md)，产品行为见 [PRODUCT.md](PRODUCT.md)，模块作者契约见 [MODULE_SPEC.md](MODULE_SPEC.md)，
> 运行包契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)，登记处契约见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)，
> 测试规范见 [TESTING.md](TESTING.md)（门户与路由），仓库协作规则见 [AGENTS.md](AGENTS.md)。

## 一、分层与依赖方向

依赖箭头只有一种画法：

```text
presentation ──▶ core ◀── adapters
                   ▲
              main（组合根，装配全部）
```

| 层 | 干什么 | 禁令 |
| --- | --- | --- |
| `core/` | 定义抽象（`ports.rs`、`api.rs`）+ 编排业务（会话、协作状态机、引擎、信封解析） | 不读文件（`std::fs`）、不发网络（ureq）、不碰 stdin/stdout——一切机制下沉适配层 |
| `adapters/` | 实现 core 的端口；可引用外部库（ureq / serde_yaml / windows-sys / libc） | 只依赖 core，**永不反向**；不做装配决策 |
| `presentation/` | 渲染事件、收集输入（CLI 与 Web 并列） | 只依赖 **core 的入站能力面**（`core::api`）；**永不接触端口对象，也拿不到 `Core` 本身** |
| `main.rs` | 组合根：`new` 出所有适配器并注入 | 除装配外无业务 |

推论：

- 「用哪个供应商/模型」是**策略**，在 core 的解析链里决定；「怎么建通道」是**机制**，在 adapters。两者不互换。
- 出站依赖由 **core 定义端口、adapters 实现**；入站依赖由 **core 定义能力接口（`core::api`）、呈现层依赖**。
  两侧都是依赖倒置，只是箭头方向不同——**核心不定义"前端接口让别人实现"**。
- 呈现层拿到的是 `Ops`（四个按角色的能力接口 + 事件台），不是 `Core`，也不是任何锁。

## 二、端口：只画在 IO 与可替换点上

端口 = `core/ports.rs` 里的 trait。判据只有一条：**这里有 IO，或者这里有可替换的实现**。
纯逻辑（信封解析、引擎循环、协作状态派生、提示词渲染）**刻意不抽象**——它们没有 trait。

| 端口 | 职责 | 适配层实现 |
| --- | --- | --- |
| `Chat` | 一次模型会话：收消息列表（可带**工具声明**）回 `Completion`（正文 + 结束原因 + 原生工具调用）；`on` 逐片回调，返回 `false` 即要求中止 | `HttpChat`（测试 `FakeChat`） |
| `ChatGateway` | 建通道（含核心通道与回落告知）；**不选择**模型；实测一条通道支不支持原生工具调用 | `HttpGateway`（无可用模型时回落 `DemoGateway`；探测发两条最小请求对比） |
| `SettingsStore` | 登记处持久化（providers / models / settings / agents 四个 yaml） | `YamlSettingsStore` |
| `ModelCatalog` | 列出一条通道当前可用的模型名 | `HttpModelCatalog` |
| `ModuleSource` | 模块清单来源（扫描 `modules/`） | `FsModules` |
| `PackageSource` | 运行包库来源（扫描依赖文件夹 `runtimes/`） | `FsPackages` |
| `Workspace` | 一次工作的 work 目录、各 agent 沙箱、文件清单与寻址根 | `FsWorkspace` |
| `SysIo` | 内置文件工具的读写机制（读严格 UTF-8、非法字节如实标注；写一律 UTF-8） | `FsSysIo` |
| `HistoryStore` | 会话历史：一个会话一个目录（meta + 事件流水） | `FsHistory` |
| `PromptSource` | 提示词册加载（`prompts.yaml`） | `YamlPrompts` |
| `ToolRunner` | 外部工具进程（围栏安装、拉起、stdin 送参、超时杀树、截断） | `ProcTools`（守门进程 = 本程序的 `--fence-run` 模式） |
| `EnvelopeRepair` | 手写信封不合法时的**无歧义**补救（改了字段含义就是错；拿不准就返回不修） | `UnambiguousRepair`（转义字符串里的裸控制字符 + 补上扫描器算出的收尾括号；断在字符串中间不修，一段回复里起了两段信封不修——补哪一段都是猜；调用方中止的生成一律不修） |
| `FenceHost` | 围栏授权的释放（删除会话时请求一次撤销） | `confine::FenceHostAdapter`（本平台无该机制时为空操作） |
| `Log` | 运行日志（三级） | `FileLog`（测试 `NoopLog`） |

新增端口前先问一句：**这是 IO 或可替换点吗**？不是就别加 trait。


## 三、模块地图与入站契约

这两块是**查阅型细则**，拆出去只有一份：

- 逐个文件讲 `core/` / `adapters/` / `presentation/` 各干什么：[docs/architecture/module-map.md](docs/architecture/module-map.md)。
- 呈现层入站契约（能力接口、事件台、命令/事件规则）与机器可读的 HTTP 路由目录：[docs/architecture/contracts.md](docs/architecture/contracts.md)。

**路由表由契约测试机器比对**（`src/tests/routes.rs` 直接读 `docs/architecture/contracts.md`）：
表与 `presentation/routes.rs` 的 `ROUTES` 对不上就是测试失败。

## 四、运行日志（Log 端口）

- core 定义 `Log`（`info`/`warn`/`error`），**只调用**；文件、时间戳、目录机制在 adapters。
- 关键节点必须埋点：通道降级、HTTP 失败、会话动作失败、装配失败、工具执行异常。
- 适配层实现（`adapters/log.rs`）：每次运行在根目录 `logs/` 下按时间戳建一个 `.log` 文件；`logs/` 不入库。
- 组合根创建唯一的 `FileLog` 并注入 core 与呈现层；测试用 `NoopLog`。
- 目的：出问题时**看日志定因**，不靠推理猜。

## 五、提示词册（prompts.yaml）

- **所有发给 LLM 的提示词一律写入 `prompts.yaml`**，禁止硬编码进代码；改文案只改册子。
- 占位符 `{{key}}`；渲染器在 `core/prompt.rs`（纯逻辑）；文件加载经 `PromptSource` 端口在适配层。
- **缺文件 / 缺键 / 缺变量 = 报错暴露**，禁止静默兜底文案。
- 文案的注入方式与端口一致：随环境对象传入（沙箱/工具环境/引用改写器），而不是让纯逻辑自己去读文件。
- 路径类占位符（`{{work_root}}` 等）由 core 在运行时替换成**真实根目录**后才交给 AI——仓库里永远不出现机器路径。

册子结构（`prompts.yaml`）：

| 段 | 键 | 用途 |
| --- | --- | --- |
| `core` | `chat_protocol` | 讨论约定（随首轮提示词注入，可自由演化） |
| | `refs.foreign_sandbox` / `refs.collab_sandbox` | `@` 引用越权与协作场景的如实说明 |
| | `discuss.opener` / `discuss.step` / `discuss.autonomy_note` | 讨论首轮、轮转、小组自裁说明 |
| | `synthesize.system` / `synthesize.user` | 整理方案 |
| | `execute.user` | 执行任务 |
| | `review.system` / `review.user` | 验收 |
| | `rerun.user` | 返工 |
| | `slate.system` / `slate.user` | 代拟名单 |
| | `suggest_models.*` | 模型推荐（单 agent / 协作两种说法） |
| | `agent.system` | agent 职责提示词骨架（模块 `system` 合成 + 内置工具说明 + 外部工具清单 + 模块工具参数段） |
| | `sys_tools` | 内置工具说明块（含本 agent 的真实根目录、模块目录、`{{tool_params}}` 参数签名与 `{{patch_guide}}`） |
| | `patch_guide` | 自由格式补丁的写法（每块以 `*** End File` 收尾、SEARCH 要整行一致、一次可多块、整体原子） |
| | `tool_calling_envelope` / `tool_calling_native` | 工具调用约定**两套，互斥**：一个通道只用一套，由通道形态决定注入哪套（同时教会让模型在正文里讲解参数而被误判成调用） |
| | `builtin_tools` | 内置三件套的**参数契约**：模型侧说明与调用校验的唯一来源（不写进代码） |
| | `no_agents` / `no_model` / `no_module_dirs` / `no_module_tools` / `no_module_tool_params` | 空态说法 |
| | `module_tool_params_header` | 模块工具参数段的小标题（模块在 `module.yaml` 里声明了 `params` 时出现） |
| | `tool_texts.*` | **运行时回执**：路径校验、参数不符（说事实 + 回发工具签名）、内置四件套回执与行区间/截断/编码标注、edit 的找不到（含"只差空白"提示）与多处命中、patch 的解析失败（缺 End File / 缺路径 / 空 SEARCH…）与"第几块为什么、整体没写"、write 的"没读过/读后又被改/只读到一部分"三种拒绝、外部工具分派的三类失败、工具超限、**信封不合法四类**（未闭合"还差什么"/"起了两段" / 裸控制字符 / 语法错 / 字段不合法）与"已修复后执行"/"输出被长度截断"的如实标注、给模型看的清单骨架、追加在回复行末尾的 `（已停止）` / `（本段被输出长度截断）` |

- **界面通知**（`[建组]`、`[上限]` 这类）是呈现层文案，**不属于**提示词册。
- **不进册子的两类**（有意留在代码里）：①**会被解析的转录锚点**（`[轮次 N]`、`[用户:需求]`、`[代拟] …`、`[id:tag]` 等，`collab_state` 与回档定位要读它们，改文案等于改状态机）；②**只给用户看的呈现层文案**（各类 `SessionEvent::Notice`、工具轨迹行的成败字样、面向界面/CLI 的 `Err`）。
- 运行时回执之所以进册子：它们会成为模型下一轮的输入，属于提示词。

## 六、状态与落盘契约

**布局**（机制口径）：

```text
session/<工作名>/
  meta.yaml          # 身份与选型：形态、agent 名单、模块、模型、需求、执行档位（exec 段）
  transcript.jsonl   # 只追加的事件流水
  work/              # 本次工作共享区（用户投喂与成品）
  <agent实例名>/      # 该 agent 的私有沙箱
```

- **转录即状态**：流水只追加；回档**只追加一条 `{"type":"rewind"}` 记录**，不物理删行；会话内容 = 回放到最后一个截断点。
- **转录行的稳定 id**：一轮模型调用 = 一条行；工具调用自成一条行；id 在会话内单调、回放可复现（回档按 id 定位）。
- **流式增量是短暂事件**：`delta` / `tool_call` 不落盘；历史只记定稿后的行。
- **工具调用形态**（`ToolMode`：`envelope` 手写信封 / `native` 原生调用）是**登记处的事实**（`models.yaml` 的 `tools`，缺省 envelope），
  **不钉在会话里**：每次生成前按登记处重新解析——变了就按重建路径就地刷新系统提示并给用户一句通知，没变什么都不做。
  **两套形态互斥**：envelope 不声明工具、只解析正文里的信封；native 只把工具声明发给供应商、不解析信封
  （正文里出现信封时**不执行**，但如实记一条失败工具行）。系统提示始终与实际协议一致，
  回放按同一规则派生（与"改 prompts.yaml 后重建"同源）。
- **工具声明里的 `parallel` 决定并发**（内置工具在 `prompts.yaml` 的 `builtin_tools`，模块工具在 `module.yaml` 的 `tools.<名字>`；
  缺省 false = 独占）：一次回复里的**连续**可并发调用合成一批并发跑，其余各自独占（写入类因此是批次之间的屏障）；
  工具行、结果消息与账本合并**一律按原始调用顺序**——并发只影响执行，不影响上下文里的顺序。
- **观察账本**（`systool::Observations`）是**进程内状态，不落盘**：记"本次会话完整读过 / 由核心写过哪些文件、当时的内容指纹"，
  只用于一处决策——整份覆盖（`write`）要不要放行。回档时清空（那段读取证据随转录一起被截掉），
  按落盘重建的会话从空账本开始（模型重新读一遍即可，宁可多读一次也不凭记忆覆盖）。
- **行上的判定走结构化字段**：例如「信封缺失、按发言原文收录」的降级行带 `degraded: true`，呈现层据此做样式——**不匹配行文本里的说明文案**（改文案不得影响行为）。列的语义同理（`tool` 视图、稳定 `id`）都挂在字段上。
- **一次模型回复在上下文里是「一条助手消息 + N 条结果」**：原生通道下助手消息带 `tool_calls`（一次回复多个调用都挂在这一条上），
  每条结果用 `role:"tool"` + `tool_call_id` 回应；手写信封通道下助手消息就是模型原文、结果当用户消息发回
  （手写信封也能一次发多个调用：写在同一个信封的 `calls` 数组里，两种形态互斥）。
  **两种形状由唯一的构造函数（`engine::reply_msgs`）按当前形态产出**，实时与回放都只走它——
  所以"重建上下文必须与实时逐条一致"是结构保证，不靠两边各自小心；形态切换时旧消息自动被表达成新形状（事实留在转录里，切回去还能还原）。
- **转录行带 `reply`**（= 该回复第一行的稳定 id）：重建按它把同一次回复的工具行归成一组，回档也按它**原子**截断
  （截在一次回复中间会留下"孤儿工具结果"，而协议要求结果紧跟发起它的助手消息）。行分组因此不靠"相邻行猜"——
  那正是曾经把实时与重建拆开的地方。
- `meta.yaml` 的 `agents` 是名单的**唯一真相**（代拟路径在用户确认名单那一刻写回）。
- `meta.yaml` 的 `exec` 段是**执行选型**的唯一真相：档位（`tier` = 本机 / 虚拟机）、虚拟机基础根、能力定版（`pins`）、是否放行出站网络；
  缺这段的 `meta.yaml` 按默认读回（本机档、不定版、不联网）。执行计划本身（`core/exec.rs` 的 `ExecPlan`）**从不落盘**——它含真实路径，只在运行时派生。
- 会话的**旁路配置记录**（`{"type":"config"}`）只在编辑提交时追加：供呈现与审计，**不进模型上下文**，回放与状态派生都跳过它。
- `core/fence.rs` 是工具进程围栏的**策略**（可达范围 = 共享区 + 自己的私有沙箱 + 自己的模块目录 + 用户显式授权的只读根 `ro`、断网、工作目录），
  `ro` 来自 `.home/settings.yaml` 的 `fence_read`（默认空）：**只读位由各平台机制落实**（Landlock 只读位 /
  seatbelt `file-read*` / Windows `RIGHTS_RO`），且只授给该 agent 自己的容器身份——不能像解释器基线那样授给共享组。
  机制在 `adapters/confine/`：外层拉起的**守门进程**（本程序 `--fence-run` 模式）按平台把围栏装进真正的工具进程
  ——Linux Landlock、macOS seatbelt、Windows AppContainer（先建容器 profile，再按 agent 派生容器 SID 与目录 ACL 授权，
  不给 capability 即断网）+ Job Object（进程树）；Windows 的目录授权由外层进程一次性做好（`confine::prepare_fence`）并记在内存台账里。
  装不上就**如实降级**（启动时自检并报告能力等级，绝不假装有）。命令行是守门进程的内部协议，模块作者与用户都不接触。
  **未授权不等于无围栏**：授权与否只决定"路径级围栏装不装"（要写目录 ACL），进程树围栏、资源上限与
  环境白名单在两种时段都生效；启动报告因此把**本机能力**与**本次实际**分两行说清，不让人误读。
  机制验证分三态（`confine::verify` / `FenceVerdict`；`--fence-verify` 是它的机器可读入口，探针据此驱动）：
  `Enforced`（装上了）/ `EnvUnavailable`（本机不允许：内核不支持、私有 ABI 失效、系统拒绝建容器）/
  `Broken`（自检已确认机制有效却仍装不上 = 我们写错了）。前两态如实降级照跑，**`Broken` 在未授权时段拒绝执行**
  （命令不落进程，回执用册子里的固定说法）——把"我们写错了"当成降级吞掉，等于用户以为有围栏、实际什么都没有。
  入参是**扁平 JSON**（`confine::FenceJob`）= 围栏策略字段 + `prepared`：后者说清外层有没有做完本机授权。
  Windows 的容器要先有读放行与落点才可能真跑起来，所以没授权时守门进程直接按无围栏执行——不去试一个注定
  读不到模块目录与解释器的容器；容器起不来也一样如实报出原因再降级。
  工具进程的环境走**白名单**（`confine::fence_env`，外层滤好后传下去）：不继承父进程环境（密钥与无关凭据不进工具进程），
  `HOME` / `TEMP` / `USERPROFILE` / `LOCALAPPDATA` 一律落到该 agent 的私有沙箱；Windows 建 AppContainer 进程
  需要 `LOCALAPPDATA` 在场（缺了它 `CreateProcessW` 报 `os error 203`，容器会静默降级成无围栏）。
  容器 profile **一个 agent 一个**（跨会话复用，数量有界）：守门进程是唯一建它的地方，建成即写进
  `.home/fence-grants.json` 台账；`--fence-clean` 先按台账精确回收（撤 ACE + 删 profile），再按
  `Solomni.Agent.` 前缀扫掉整族遗留 profile（探针、台账被删、旧版本建的都在这一扫里）。
- `core/packages.rs` 是运行包契约与包库事实（校验、去重、系统路径冲突预检、能力索引），
  `core/exec.rs` 是执行档位与执行计划派生；两者都是纯逻辑，目录遍历在 `PackageSource` 适配层。契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)。
- 登记处四份 yaml 的字段与读写规则见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)。
- 内存与落盘不一致时**以流水为准**（可回放、可重建）。

## 七、可测性（架构约束）

测试的层级、替身语义、端口契约矩阵、质量门禁、缺口账与执行入口全部由 [TESTING.md](TESTING.md)（门户与路由）
与 `docs/testing/` 下的细则规定；本节只列架构对可测性的硬约束，不重复测试规范。

- 任意需要 IO 或存在可替换实现的机制必须通过 `core/ports.rs` 中的端口注入；core 不直接依赖真实模型、网络、文件系统、时钟、随机数或外部进程。
- 纯逻辑（信封解析、协作状态派生、提示词渲染、路径寻址等）不为测试强行增加 trait，直接以纯函数测试；端口只放在真实边界和确有替换价值的点上。
- 端口的输入、输出、错误、取消、超时、重复调用和资源清理语义属于架构契约：生产适配器与测试替身必须遵守同一份契约。
- 端口不能为了方便测试暴露生产实现的内部状态；需要观察交互时，通过测试替身的记录能力或公开的行为结果观察。
- 组合根测试当前使用的内存装配（`InMemory*`、`VecSource`、`ScriptGateway`、`NoopLog` 等）集中在 `src/tests/doubles.rs`；
  新增替身用能表达职责的名称，并在测试基础设施中集中维护。
- 质量门禁（格式、编译、Clippy、依赖重复、测试结构冗余）与业务测试是两类事实，分别记录，质量失败不能被业务测试通过抵消。
- 代码冗余检查不改变分层与端口设计，也不以增加 trait、包装层或测试用例为目标；发现重复时先判断是否同一职责，再决定合并、保留或记录原因。

## 八、跨平台机制

- 路径一律用 `PathBuf`/`Path` 组件拼接：**禁止把 `/` 或 `\` 写进字符串再拼**（分隔符交给运行环境）。
- **对外**（提示词、工具参数、回执、API）一律用 `/` 书写形式：Windows 的反斜杠在 JSON 字符串里是**非法转义**（`\A`、`\S` 之类），模型据此拼出的参数会直接解析失败。
- 编码：读严格 UTF-8、非法字节如实标注（**不猜编码**）；写一律 UTF-8；为工具子进程强制 UTF-8 环境。
- 不假设平台：不写死盘符、不假设 shell（工具命令由模块作者声明）。
- 围栏按平台给不同机制（`adapters/confine/` 一个平台一个文件），能力等级如实上报；某平台没有接入的部分**就是没有**——文档与界面都照实说，不用夸张的措辞补齐。
