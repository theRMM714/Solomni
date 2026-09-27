# 架构与开发规则（ARCHITECTURE）

> 改代码前必读。本文是**分层的唯一权威**：谁依赖谁、端口画在哪、机制放哪一层、日志与提示词册怎么用、落盘契约长什么样。
> 理念见 [PHILOSOPHY.md](PHILOSOPHY.md)，产品行为见 [PRODUCT.md](PRODUCT.md)，模块作者契约见 [MODULE_SPEC.md](MODULE_SPEC.md)，
> 运行包契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)，登记处契约见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)，
> 测试规范见 [TESTING.md](TESTING.md)（门户与路由），仓库协作规则见 [AGENTS.md](AGENTS.md)。

## 一、分层与依赖方向

依赖箭头只有一种画法（每层只指向它的下层）：

```text
cli / web ──▶ capabilities（含协调业务 conductor）──▶ kernel
（main = 组合根，装配全部；各能力的 `detail` 与 kernel 的实现只在这里被 new 出来，它们不认识业务编排）
```

| 层 | 干什么 | 禁令 |
| --- | --- | --- |
| `capabilities/conductor/` | **协调业务**：会话中心（会话在世表、命令队列、运行态）、生成驱动、跨能力用例与跨会话回档编排。它与别的能力**平级**，只经各能力的 `api` 编排，不持任何别人的端口 | 不读文件（`std::fs`）、不发网络（ureq）、不碰 stdin/stdout——一切机制下沉各能力的 `detail/` |
| `entry/` | **入口层共用机制**（产品根规范化）；只有组合根 / `diagnostics` / `guard` 能用 | 属于入口层；**任何能力都不许依赖它** |
| `cli/` + `web/` | **前端（交付机制）**：各渠道一个顶层目录，完全分开——传输（argv/stdout vs HTTP/SSE）、路由、**纯渲染**。**不是业务能力**（无状态、无不变式） | 只依赖 **入站能力面**（各能力的 `::api`）；**永不接触端口对象，也拿不到 `Core` 本身**；**两者之间互不依赖** |
| `capabilities/` | **业务能力**：按业务功能垂直切分。每个能力有 `api`（入站契约：trait + DTO）/ `service`（**本能力的状态与用例**，实现 `api` 的 trait；别处只持 `dyn` 面）/ `ports`（出站端口，**只由定义它的能力持有**）/ `domain`（纯逻辑）/ `detail`（细节实现，**只有组合根能构造**） | **业务之间只经对方的 `api`**；不反向依赖 `core` / `adapters` / `presentation`（迁移期残留记为基线豁免，见 §九（业务边界与硬要求） §四） |
| `kernel/` | **机制型业务**：无领域语义、无领域状态的机制（运行日志端口、宿主探测、生成中作业的取消表、跨业务共享的事实类型、路径书写）；形状与别的能力一致（`api` / `ports` / `domain` / `detail`） | **不依赖任何人**（不认识能力 / presentation / entry）；不放有领域语义的类型 |
| 入口层（`main.rs` + `diagnostics/` + `guard/`） | **组合根**（`main.rs`：`new` 出所有适配器并注入）+ **机器可读探针**（`diagnostics/`：`--doctor` / `--https-check` / `--print-routes` / `--print-fence-env` / `--fence-verify`）+ **围栏守门进程**（`guard/`：`--fence-run` / `--fence-clean`，**第二个程序入口**） | 它依赖所有人，**任何人都不许依赖它**（门禁判定）。除装配与探针外无业务 |

推论：

- 「用哪个供应商/模型」是**策略**，在 `registry`（登记处能力）的解析链里决定；「怎么建通道」是**机制**，在 llm 的 `detail`。两者不互换。
- 出站依赖由**各能力定义端口**（`capabilities/<能力>/ports.rs`）、**各能力（与 kernel）的 `detail` 实现**；入站依赖由**各能力定义能力接口**（`<能力>::api`）——协调业务的 `api::Ops` 只把它们与它自己的两个契约**组装**成一份交给呈现层（批次 18）。
  两侧都是依赖倒置，只是箭头方向不同——**能力不定义"前端接口让别人实现"**。
- 呈现层拿到的是 `Ops`（各能力的能力接口 + 核心自己的两个 + 事件台），不是 `Core`，也不是任何锁。

## 二、端口：只画在 IO 与可替换点上

端口 = 各能力的 `capabilities/<能力>/ports.rs`（业务/机制边界）与 `kernel/`（无领域语义的机制）里的 trait。判据只有一条：**这里有 IO，或者这里有可替换的实现**。
纯逻辑（信封解析、引擎循环、协作状态派生、提示词渲染）**刻意不抽象**——它们没有 trait。

| 端口 | 职责 | 适配层实现 |
| --- | --- | --- |
| `Chat`（`capabilities/llm/ports.rs`） | 一次模型会话：收消息列表（可带**工具声明**）回 `Completion`（正文 + 结束原因 + 原生工具调用）；`on` 逐片回调，返回 `false` 即要求中止 | `HttpChat`（测试 `FakeChat`） |
| `ChatGateway` | 建通道（含核心通道与回落告知）；**不选择**模型；实测一条通道支不支持原生工具调用。定义在 `capabilities/llm/ports.rs`，**只由 llm 的 `service.rs` 持有**（R12）；别人经 `llm::api::Llm` 要通道 | `HttpGateway`（无可用模型时回落 `DemoGateway`；探测发两条最小请求对比） |
| `SettingsStore` | 登记处持久化（providers / models / settings / agents 四个 yaml）。定义在 `capabilities/registry/ports.rs` | `YamlSettingsStore` |
| `ModelCatalog` | 按**端点与密钥**列出一条通道当前可用的模型名。`capabilities/llm/ports.rs`，**只由 llm 的 `service.rs` 持有**（R12） | `HttpModelCatalog` |
| `ModuleSource` | 模块清单来源（扫描 `modules/`）。**已随能力搬出**：`capabilities/workspace/ports.rs` | `FsModules` |
| `PackageSource` | 运行包库来源（扫描依赖文件夹 `runtimes/`）。**已随能力搬出**：`capabilities/workspace/ports.rs` | `FsPackages` |
| `Workdirs` | 一次工作的 work 目录、各 agent 沙箱、文件清单与寻址根。`capabilities/workspace/ports.rs`，**只由 workspace 的 `service.rs` 持有**（R12） | `FsWorkspace` |
| `SysIo` | 内置文件工具的读写机制（读严格 UTF-8、非法字节如实标注；写一律 UTF-8）。`capabilities/tools/ports.rs`，**只由 tools 的 `service.rs` 持有**（R12） | `FsSysIo` |
| `HistoryStore` | 会话历史：一个会话一个目录（meta + 事件流水）。`capabilities/session/ports.rs`，**只由 session 的 `service.rs` 持有**（R12）；别人经 `session::api::History` 读写 | `FsHistory` |
| `PromptSource` | 提示词册加载（`prompts/`）。定义在 `capabilities/prompt/ports.rs` | `YamlPrompts` |
| `SystoolsSource` | 工具总表与角色表的加载（`systools/tools.yaml` + `roles.yaml`）。定义在 `capabilities/tools/ports.rs` | `YamlSystools` |
| `ToolRunner` | 外部工具进程（围栏安装、拉起、stdin 送参、超时杀树、截断）。`capabilities/tools/ports.rs`，**只由 tools 的 `service.rs` 持有**（R12）；别人经 `tools::api::ToolExec` 跑工具 | `ProcTools`（守门进程 = 本程序的 `--fence-run` 模式） |
| `EnvelopeRepair`（`capabilities/llm/ports.rs`） | 手写信封不合法时的**无歧义**补救（改了字段含义就是错；拿不准就返回不修） | `UnambiguousRepair`（转义字符串里的裸控制字符 + 补上扫描器算出的收尾括号；断在字符串中间不修，一段回复里起了两段信封不修——补哪一段都是猜；调用方中止的生成一律不修） |
| `FenceHost` | 围栏授权的释放（删除会话时请求一次撤销）。`capabilities/tools/ports.rs`，**只由 tools 的 `service.rs` 持有**（R12） | `confine::FenceHostAdapter`（本平台无该机制时为空操作） |
| `Log` | 运行日志（三级） | `FileLog`（测试 `NoopLog`） |
| `HostProbe` | 宿主能力探测（**只问事实**：路径存在性、PATH 上的可执行文件、本机虚拟化能力；不执行、不安装、不写）。**在 `kernel/ports.rs`**（无领域语义，执行档位与自检共用；实现在 `kernel/detail/host_probe.rs`） | `HostProbeAdapter`（测试 `FixedProbe`） |

新增端口前先问一句：**这是 IO 或可替换点吗**？不是就别加 trait。


## 三、模块地图与入站契约

这两块是**查阅型细则**，拆出去只有一份：

- 逐个文件讲 `capabilities/`（含 `conductor/`）/ `kernel/` / `entry/` / `cli/` / `web/` 各干什么：[docs/architecture/module-map.md](docs/architecture/module-map.md)。
- 呈现层入站契约（能力接口、事件台、命令/事件规则）与机器可读的 HTTP 路由目录：[docs/architecture/contracts.md](docs/architecture/contracts.md)。
- 系统工具总表、角色表与"谁能用哪些工具"（含越权校验与提示词按角色分配）：[docs/architecture/tools-and-roles.md](docs/architecture/tools-and-roles.md)。
- 协作如何从讨论走到交付（审查关卡、任务链、子会话、验收）：[docs/architecture/task-chain.md](docs/architecture/task-chain.md)。
- 提示词册（`prompts/`）的结构与键清单：[docs/architecture/prompts.md](docs/architecture/prompts.md)。
- 重构的迁移账（业务边界判据、能力清单、批次与销账）：§九（业务边界与硬要求）。

**路由表由契约测试机器比对**（`src/tests/routes.rs` 直接读 `docs/architecture/contracts.md`）：
表与 `web/routes.rs` 的 `ROUTES` 对不上就是测试失败。

## 四、运行日志（Log 端口）

- `kernel/ports.rs` 定义 `Log`（`info`/`warn`/`error`），**只调用**；文件、时间戳、目录机制在 `kernel/detail/file_log.rs`。
- 关键节点必须埋点：通道降级、HTTP 失败、会话动作失败、装配失败、工具执行异常。
- 机制实现（`kernel/detail/file_log.rs`）：每次运行在根目录 `logs/` 下按时间戳建一个 `.log` 文件；`logs/` 不入库。
- 组合根创建唯一的 `FileLog` 并注入协调业务与呈现层；测试用 `NoopLog`。
- 目的：出问题时**看日志定因**，不靠推理猜。

## 五、提示词册（prompts/）

- **所有发给 LLM 的提示词一律写入 `prompts/`**，禁止硬编码进代码；改文案只改册子。
- 占位符 `{{key}}`；渲染器在 `capabilities/prompt/domain/prompt.rs`（纯逻辑）；文件加载经 `PromptSource` 端口在适配层。
- **缺文件 / 缺键 / 缺变量 = 报错暴露**，禁止静默兜底文案。
- **册子只由提示词能力持有一次**（`capabilities/prompt/service.rs`）：组合根装载后把 `Arc<dyn Prompt>` 注入协调业务，
  协作会话与它**共享同一份**（不再每个会话克隆整本册子）。
- **别的能力不点字段路径**：按名字取段（`Prompt::text` / `Prompt::render` + `Segment`），
  或拿走两块**共享记录**（`Prompt::tools()` 的 `tool_texts` / `Prompt::refs()`，都是 `Arc`）。
  "哪个回合发哪几段"的**组装留在各业务**（身份块归 `session`、工具说明归 `tools`、清单文本归 `registry` / `workspace`）
  ——prompt 只给"段"，不替它们拼。
- 路径类占位符（`{{work_root}}` 等）由协调业务在运行时替换成**真实根目录**后才交给 AI——仓库里永远不出现机器路径。

提示词册的**文件与键清单**（每份文件里有什么键、每个键干什么）只有一份：
[docs/architecture/prompts.md](docs/architecture/prompts.md)。

- **界面通知**（`[建组]`、`[上限]` 这类）是呈现层文案，**不属于**提示词册。
- **不进册子的两类**（有意留在代码里）：①**行身份是结构化字段**（`LineView` 的 `speaker` / `verb` / `kind`）：
  转录行不靠"标签写成什么样"来认，`LineView::render()` 是"字段 → 文本"的唯一拼法——`collab_state` 派生与
  回档定位都读字段，改文案不再等于改状态机；②**只给用户看的呈现层文案**（各类 `SessionEvent::Notice`、
  工具轨迹行的成败字样、面向界面/CLI 的 `Err`）。
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
  回放按同一规则派生（与"改 prompts/ 后重建"同源）。
- **工具声明里的 `parallel` 决定并发**（内置工具在 `systools/tools.yaml` 的 `tools`，模块工具在 `module.yaml` 的 `tools.<名字>`；
  缺省 false = 独占）：一次回复里的**连续**可并发调用合成一批并发跑，其余各自独占（写入类因此是批次之间的屏障）；
  工具行、结果消息与账本合并**一律按原始调用顺序**——并发只影响执行，不影响上下文里的顺序。
- **观察账本**（`systool::Observations`）是**进程内状态，不落盘**：记"本次会话完整读过 / 由核心写过哪些文件、当时的内容指纹"，
  只用于一处决策——整份覆盖（`write`）要不要放行。回档时清空（那段读取证据随转录一起被截掉），
  按落盘重建的会话从空账本开始（模型重新读一遍即可，宁可多读一次也不凭记忆覆盖）。
- **行上的判定走结构化字段**：例如「信封缺失、按发言原文收录」的降级行带 `degraded: true`，呈现层据此做样式——**不匹配行文本里的说明文案**（改文案不得影响行为）。列的语义同理（`tool` 视图、稳定 `id`）都挂在字段上。
- **一次模型回复在上下文里是「一条助手消息 + N 条结果」**：原生通道下助手消息带 `tool_calls`（一次回复多个调用都挂在这一条上），
  每条结果用 `role:"tool"` + `tool_call_id` 回应；手写信封通道下助手消息就是模型原文、结果当用户消息发回
  （手写信封也能一次发多个调用：写在同一个信封的 `calls` 数组里，两种形态互斥）。
  **两种形状由唯一的构造函数（`session::api::reply_msgs`）按当前形态产出**，实时与回放都只走它——
  所以"重建上下文必须与实时逐条一致"是结构保证，不靠两边各自小心；形态切换时旧消息自动被表达成新形状（事实留在转录里，切回去还能还原）。
- **会话参数与对话分开**：身份、工作环境（真实根目录）、调用约定是**参数**（`session::SessionParams`），
  与登记处/提示词册一起在**每次调用**现场渲染；对话（`dialogue`）里只有真正发生过的事——用户说了什么、
  模型答了什么、调了什么工具。请求由**唯一一处**装配（`engine::assemble`）：`[身份][本回合工具面][对话…][本回合提示]`。
  参数因此不冒充对话：回档只截对话、压缩只算对话，改一个参数（例如登记处里的工具形态）也只改那一格，不必重建会话。
- **转录行带 `reply`**（= 该回复第一行的稳定 id）：重建按它把同一次回复的工具行归成一组，回档也按它**原子**截断
  （截在一次回复中间会留下"孤儿工具结果"，而协议要求结果紧跟发起它的助手消息）。行分组因此不靠"相邻行猜"——那正是把实时与重建拆开的做法。
- `meta.yaml` 的 `agents` 是名单的**唯一真相**（代拟路径在用户确认名单那一刻写回）。
- `meta.yaml` 的 `exec` 段是**执行选型**的唯一真相：档位（`tier` = 本机 / 虚拟机）、虚拟机基础根、能力定版（`pins`）、是否放行出站网络；
  缺这段的 `meta.yaml` 按默认读回（本机档、不定版、不联网）。执行计划本身（`capabilities/workspace/` 的 `ExecPlan`）**从不落盘**——它含真实路径，只在运行时派生。
- 会话的**旁路配置记录**（`{"type":"config"}`）只在编辑提交时追加：供呈现与审计，**不进模型上下文**，回放与状态派生都跳过它。
- 出站模型调用的参数由**核心**决定、随端口传下去：`llm::api::LlmOpts{stream, timeout_secs}` 与 `CompleteOpts` 的对应字段，
  取值来自**全局设置**（`streaming` / `llm_timeout_secs`），讨论、执行、验收与单 agent 共用同一份判据（`Core::llm_opts`）。
  `Output` 只管回包形状：设置是流式的**上限**，调用方可在本次放弃流式。
  调用失败**不是**模型的回复：`Completion.error` 与正文分离，上层据此发 `Notice` 并**中断本轮**（不落任何转录行），
  会话保持可继续——错误文本若被当成发言吸收，按转录派生的「轮到谁」就歪了。
- `capabilities/tools/` 里的 `fence` 是工具进程围栏的**策略**（可达范围 = 共享区 + 自己的私有沙箱 + 自己的模块目录 + 用户显式授权的只读根 `ro`、断网、工作目录），
  `ro` 来自 `.home/settings.yaml` 的 `fence_read`（默认空）：**只读位由各平台机制落实**（Landlock 只读位 /
  seatbelt `file-read*` / Windows `RIGHTS_RO`），且只授给该 agent 自己的容器身份——不能像解释器基线那样授给共享组。
  机制在 `capabilities/tools/detail/confine/`：外层拉起的**守门进程**（本程序 `--fence-run` 模式）按平台把围栏装进真正的工具进程
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
  Windows 的目录授权除数据边界叶子外，还给**叶子的直接父目录**一条只读属性位（`RIGHTS_STAT`，不递归不继承）：
  容器里对中间目录没有它时，`exists()` 会对一个**确实存在**的目录返回假，模块"父目录不存在就先建"的逻辑
  会一路建到盘卷根才报 `WinError 5`（真机 CI 抓到的就是 Python 的 `os.makedirs`）。只授属性位：
  能判断存在性，读不到内容、列不了目录；落点清单由 `confine::grant_targets` 统一给出，授权与撤权共用同一份。
  工具进程的环境走**白名单**（`confine::fence_env`，外层滤好后传下去）：不继承父进程环境（密钥与无关凭据不进工具进程），
  `HOME` / `TEMP` / `USERPROFILE` / `LOCALAPPDATA` 一律落到该 agent 的私有沙箱；Windows 建 AppContainer 进程
  需要 `LOCALAPPDATA` 在场（缺了它 `CreateProcessW` 报 `os error 203`，容器会静默降级成无围栏）。
  容器 profile **一个 agent 一个**（跨会话复用，数量有界）：守门进程是唯一建它的地方，建成即写进
  `.home/fence-grants.json` 台账；`--fence-clean` 先按台账精确回收（撤 ACE + 删 profile），再按
  `Solomni.Agent.` 前缀扫掉整族遗留 profile（探针、台账被删、旧版本建的都在这一扫里）。
- `capabilities/workspace/` 里的 `packages` 是运行包契约与包库事实（校验、去重、系统路径冲突预检、能力索引），
  `exec` 是执行档位与执行计划派生；两者都是纯逻辑，目录遍历在 `PackageSource` 适配层。契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)。
- 登记处四份 yaml 的字段与读写规则见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)。
- 内存与落盘不一致时**以流水为准**（可回放、可重建）。

## 七、可测性（架构约束）

测试的层级、替身语义、端口契约矩阵、质量门禁、缺口账与执行入口全部由 [TESTING.md](TESTING.md)（门户与路由）
与 `docs/testing/` 下的细则规定；本节只列架构对可测性的硬约束，不重复测试规范。

- 任意需要 IO 或存在可替换实现的机制必须通过 `capabilities/<能力>/ports.rs`（**无领域语义的机制端口在 `kernel/`**）中的端口注入；能力不直接依赖真实模型、网络、文件系统、时钟、随机数或外部进程。
- 纯逻辑（信封解析、协作状态派生、提示词渲染、路径寻址等）不为测试强行增加 trait，直接以纯函数测试；端口只放在真实边界和确有替换价值的点上。
- 端口的输入、输出、错误、取消、超时、重复调用和资源清理语义属于架构契约：生产适配器与测试替身必须遵守同一份契约。
- 端口不能为了方便测试暴露生产实现的内部状态；需要观察交互时，通过测试替身的记录能力或公开的行为结果观察。
- 组合根测试当前使用的内存装配（`InMemory*`、`VecSource`、`ScriptGateway`、`NoopLog` 等）集中在 `src/tests/doubles.rs`；
  新增替身用能表达职责的名称，并在测试基础设施中集中维护。
- **分层与依赖方向由 T0 结构审查机器判定**：层间不反向、`presentation` 只经入站能力面驱动、业务层内部不成环。
  迁移期的基线在 `tests/dependency-baseline.json`，**条目一旦不再成立即失败**（强制销账）。
- 质量门禁（格式、编译、Clippy、依赖重复、测试结构冗余）与业务测试是两类事实，分别记录，质量失败不能被业务测试通过抵消。
- 代码冗余检查不改变分层与端口设计，也不以增加 trait、包装层或测试用例为目标；发现重复时先判断是否同一职责，再决定合并、保留或记录原因。

## 八、跨平台机制

- 路径一律用 `PathBuf`/`Path` 组件拼接：**禁止把 `/` 或 `\` 写进字符串再拼**（分隔符交给运行环境）。
- **对外**（提示词、工具参数、回执、API）一律用 `/` 书写形式：Windows 的反斜杠在 JSON 字符串里是**非法转义**（`\A`、`\S` 之类），模型据此拼出的参数会直接解析失败。
  书写形式由 `kernel::path::slash` 统一给出（纯机制，与任何业务无关）。
- 编码：读严格 UTF-8、非法字节如实标注（**不猜编码**）；写一律 UTF-8；为工具子进程强制 UTF-8 环境。
- 不假设平台：不写死盘符、不假设 shell（工具命令由模块作者声明）。
- 围栏按平台给不同机制（`capabilities/tools/detail/confine/` 一个平台一个文件），能力等级如实上报；某平台没有接入的部分**就是没有**——文档与界面都照实说，不用夸张的措辞补齐。

## 九、业务边界与硬要求

> 这一节回答两个问题：**什么该独立成一个业务**，以及**所有业务必须守的硬要求**。
> 依赖方向由 T0 结构审查按 §九.7 的口径**机器判定**；当前基线为空 = 零豁免。

### 九.1 判据（两条同时成立，才配独立成一个业务）

1. **有自己的用例**：一组有领域词、有**自有协议**（输入 / 输出 / 载荷形状）、有**用户可见后果**的行为。
   **它说的概念属于某个参与方 → 就归那个参与方**（`resolve_picks` 说的是登记处自己的数据，被两处调用也不升级成业务）；
   不属于任何参与方的概念（名单、任务链）才自成业务。
2. **能指名 ≥2 个调用方**：不抽出来，这两个（或更多）能力就会各写一遍。**已实测的重复是最强证据**
   （`slate` 就是这么独立的：`conductor` 与 `collab` 从前各写一遍）；**只有一个调用方的编排不抽**，留在它自己的 `service.rs`。

**状态不参与这条判据**——它只在 §九.2 回答「这块状态归谁」。

### 九.2 状态归属（回答「归谁」，不回答「是不是业务」）

- 有参与方拥有它的不变式 → 归**那个参与方**（它的 `service.rs` 写，别人只经 `api` 读改）；
- 没有参与方拥有、且 ≥2 个参与方共同依赖 → 归**协调业务**（`conductor` 的会话在世表、命令队列与运行态）；
- 只被一个参与方用、自己又没有不变式 → 它是**派生值**，留在 owner 的 `domain/`，现算。

**协调业务可以持状态**；唯一的限制是：**它的状态只能它自己写，别人的状态只能经 `api` 拿**（R4）。

### 九.3 三分法（正交：只决定它持有什么、依赖谁）

| 类型 | 判据 | 归属与形态 |
| --- | --- | --- |
| **协调型业务** | 跨参与方的编排：有自有协议 + 用户可见后果 + 领域词 API | 独立业务；**可持状态**；向下调参与方 `api`，**参与方不得反向调它**（`conductor`） |
| **领域型业务** | 说的是自己的数据 / 自己的概念，规则与状态自成一体 | 独立业务；**只持自己的状态**；有 IO 或可替换点才有 `ports.rs` / `service.rs`（`taskchain` 就是没有端口、没有 `service` 的领域型） |
| **机制型（`kernel`）** | 没有领域语义 | `kernel/`：**领域词测试**——它不需要知道什么是回合 / 回复 / 工具执行；有机制状态与机制端口，只被依赖 |

### 九.4 反例：什么不配切成业务

- **纯机制**（线程、锁、定时、文件句柄、HTTP 客户端）→ `kernel` 或所属能力的 `detail/`；
- **纯逻辑 / 派生**（解析器、状态派生、提示词渲染）→ 所属业务的 `domain/`，**不加 trait**；
- **纯交付**（HTTP 路由、终端渲染）→ `cli/` + `web/`，**不是能力**；
- **只服务单一调用方的脚本 / 编排** → 它自己的 `service.rs`；
- **DTO 中转站**（把两个能力的类型拼一起的适配业务）→ 新的水平层，禁止。

### 九.5 粒度上限

- 一个能力的**实现文件**（`service/**` / `domain/**` / `detail/**`）≤ **800 行**：
  `conductor/service/` 与 `collab/service/` 都按方法族分块，当前最大的 `work.rs` 641 行；
- 一个能力的 `api.rs` ≤ **20 个方法**——按「一个调用角色的面」计数：同一批用例面向两类调用方（呈现层走命令队列、其它能力同步读事实）时会各成一个 trait，分别计数；
  `registry` 是已知的宽面，**它仍是一个能力**——判据是 §九.1，不是方法数；
- 测试文件按 R10 的上限（2000 行）；
- **两处已知超标、待拆**：`conductor/api.rs`（1551 行：三个入站 trait + `Ops` 组装 + 队列代理 + 用例词汇）与
  `tools/detail/confine/windows.rs`（1276 行：平台围栏后端，一个平台一个文件——按平台切而不是按行数）。

### 九.6 硬要求清单

| 编号 | 要求 |
| --- | --- |
| **R1** | 业务之间**只经对方的 `api` 交流**（trait + DTO）。不准引 `ports` / `domain` / `detail`；不准给别的能力的类型写 `impl`；`::detail` 只有入口层能碰 |
| **R2** | 依赖图**无环**，由 T0 结构审查机器判定 |
| **R3** | DIP **只画在 IO 或可替换点上**；纯逻辑刻意不抽象 |
| **R4** | **状态所有权排他**：一块状态只有一个能力写，别人只经 `api` 读改 |
| **R5** | **并发模型不变式**：单线程命令队列 + 能力间同步调用，**核心状态不加锁** |
| **R6** | **共享事实类型只属于 `kernel`**，禁止各业务复制 DTO |
| **R7** | **不留兼容层**：项目是 GREEN FIELD，迁移是搬家 + 删旧 |
| **R8** | **文档同步**：改结构必须同一次改 `ARCHITECTURE.md` / `docs/architecture/module-map.md` / 相关细则 / `AGENTS.md` 路由表 |
| **R9** | **跨平台与路径**：一律 `PathBuf` 组件拼接；对外用 `/`；不假设平台 |
| **R10** | **测试跟着业务分区走**：`capabilities/<名称>/` ↔ `src/tests/<名称>.rs`，单文件 ≤ 2000 行 |
| **R11** | **测试入口 = 生产入口**：禁止 `#[cfg(test)]` 专用语义入口 |
| **R12** | **端口只有一个持有者**：定义它的那个能力的 `service.rs`；别人只拿 `api` 面。**例外**：`kernel` 的机制端口（`Log` / `HostProbe`）是全项目共享的机制接口 |
| **R13** | **跨能力的同形重复不许存在**：要么收进唯一所有者，要么独立成一个业务（见 §九.1 第 2 条） |

### 九.7 依赖方向门禁与基线

`node run-tests.js` 的 T0 结构审查按下列判据机器判定（规则在 `run-tests.js`，豁免清单在 `tests/dependency-baseline.json`）：

- **apiOnly**：跨能力引用必须以 `::api` 结尾；任何非入口层引用别人的 `ports` 都算违规；
- **apiPorts** / **domainPorts**：`api` 与 `domain` 都不引端口；
- **foreignImpl**：不得给别的能力的类型写 `impl`；
- **coreCycles**：能力节点图无环；
- **reverse** / **presentation**：能力不反向依赖入口层或呈现层。

**当前基线为空（零豁免）**：任一判据不成立即报错；豁免条目一旦不再成立，门禁报「基线豁免已过期」强制销账。
