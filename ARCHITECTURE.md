# 架构与开发规则（ARCHITECTURE）

> 改代码前必读。本文是**分层的唯一权威**：谁依赖谁、端口画在哪、机制放哪一层、日志与提示词册怎么用、落盘契约长什么样。
> 理念见 [PHILOSOPHY.md](PHILOSOPHY.md)，产品行为见 [PRODUCT.md](PRODUCT.md)，模块作者契约见 [MODULE_SPEC.md](MODULE_SPEC.md)，
> 运行包契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)，登记处契约见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)，
> 测试规范见 [TESTING.md](TESTING.md)，仓库协作规则见 [AGENTS.md](AGENTS.md)。

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
| `Chat` | 一次模型会话：收消息列表回原文；`on` 逐片回调，返回 `false` 即要求中止 | `HttpChat`（测试 `FakeChat`） |
| `ChatGateway` | 建通道（含核心通道与回落告知）；**不选择**模型 | `HttpGateway`（无可用模型时回落 `DemoGateway`） |
| `SettingsStore` | 登记处持久化（providers / models / settings / agents 四个 yaml） | `YamlSettingsStore` |
| `ModelCatalog` | 列出一条通道当前可用的模型名 | `HttpModelCatalog` |
| `ModuleSource` | 模块清单来源（扫描 `modules/`） | `FsModules` |
| `PackageSource` | 运行包库来源（扫描依赖文件夹 `runtimes/`） | `FsPackages` |
| `Workspace` | 一次工作的 work 目录、各 agent 沙箱、文件清单与寻址根 | `FsWorkspace` |
| `SysIo` | 内置文件工具的读写机制（读严格 UTF-8、非法字节如实标注；写一律 UTF-8） | `FsSysIo` |
| `HistoryStore` | 会话历史：一个会话一个目录（meta + 事件流水） | `FsHistory` |
| `PromptSource` | 提示词册加载（`prompts.yaml`） | `YamlPrompts` |
| `ToolRunner` | 外部工具进程（围栏安装、拉起、stdin 送参、超时杀树、截断） | `ProcTools`（守门进程 = 本程序的 `--fence-run` 模式） |
| `FenceHost` | 围栏授权的释放（删除会话时请求一次撤销） | `confine::FenceHostAdapter`（本平台无该机制时为空操作） |
| `Log` | 运行日志（三级） | `FileLog`（测试 `NoopLog`） |

新增端口前先问一句：**这是 IO 或可替换点吗**？不是就别加 trait。

## 三、模块地图

### 3.1 `core/`（抽象与业务，无 IO）

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 核心层入口与 `Core` 门面：会话中心、登记处编排、运行包报告 |
| `ports.rs` | 出站端口 trait 与跨层数据结构（依赖倒置的边界；core 需要什么，由适配器实现） |
| `api.rs` | **入站契约**：四个按角色的能力接口 + `CoreHandle`（核心自有线程、命令/事件）+ `EventBus` + `JobRegistry` |
| `events.rs` | 呈现侧契约：`SessionEvent` 与介入请求的词汇（**事实**的线格式定义在这） |
| `prompt.rs` | 提示词渲染：`{{key}}` 占位替换，缺键/缺变量报错 |
| `schema.rs` | 工具参数契约（**声明在文本层**）：解析/校验/两种渲染（模型侧说明、JSON Schema） |
| `module.rs` | `module.yaml` 契约、扫描结果 `Roster`、`runtimes`/`tools` 校验、agent system 合成 |
| `packages.rs` | `package.yaml` 契约与包库事实（校验、去重、系统路径冲突预检） |
| `exec.rs` | 执行档位（`ExecSpec`）与执行计划（`ExecPlan`）派生、虚拟机档诊断 |
| `fence.rs` | 一次工具执行的围栏策略（纯数据：可达根、断网、工作目录） |
| `workspace.rs` | 工作区与沙箱的纯数据定义、寻址与越界判定 |
| `systool.rs` | 内置工具 `read` / `write` / `search` 的放行、寻址、**按声明校验参数**与回执文案 |
| `refs.rs` | 用户 `@` 引用改写成真实绝对路径 |
| `providers.rs` | 供应商/模型登记处内存形态与「模型 → 通道」解析 |
| `agents.rs` | agent 登记处、代拟名单落地与名字校验 |
| `history.rs` | 会话元信息与历史视图的内存形态 |
| `envelope.rs` | 发言信封解析（含「像工具信封但不合法」的独立信号，并判定**未闭合 / 裸控制字符 / 语法错 / 字段不合法**四类） |
| `collab_state.rs` | 「转录即状态」的协作状态派生（纯函数、可回放） |
| `collab.rs` | 协作会话状态机与前端拉模式驱动 |
| `engine.rs` | 讨论/执行/验收的引擎循环与工具循环 |
| `session.rs` | 单 agent 会话（历史自有、转录行稳定 id） |

### 3.2 `adapters/`（机制，实现 core 端口）

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 适配层出口与统一 re-export |
| `confine/` | 守门进程与平台围栏后端：`mod.rs` 装配与能力自报，`linux.rs` / `macos.rs` / `windows.rs` / `other.rs` 各平台机制 |
| `proc_tools.rs` | `ToolRunner`：守门进程拉起、stdin 送参、超时杀树、输出截断 |
| `sys_io.rs` | `SysIo`：内置工具的读写机制（UTF-8 解码、非法字节 `lossy` 标注） |
| `fs_modules.rs` | `ModuleSource`：扫描 `modules/` |
| `fs_packages.rs` | `PackageSource`：扫描 `runtimes/` |
| `fs_workspace.rs` | `Workspace`：`session/<工作名>/` 下的 work 与各 agent 沙箱 |
| `fs_history.rs` | `HistoryStore`：`meta.yaml` + `transcript.jsonl` |
| `yaml_settings.rs` | `SettingsStore`：登记处四份 yaml 的读写（见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)） |
| `yaml_prompts.rs` | `PromptSource`：加载 `prompts.yaml` |
| `endpoint.rs` | 端点补全/回落规则与进程内端点记忆（纯逻辑） |
| `http_agent.rs` | 出站 HTTP 代理构建（TLS 后端选择与超时的单点） |
| `http_chat.rs` | `Chat` / `ChatGateway`：OpenAI 兼容 `/chat/completions` |
| `model_catalog.rs` | `ModelCatalog`：OpenAI 兼容 `GET /models` |
| `fake_chat.rs` | 演示/测试通道：`FakeChat`（脚本回放）+ `DemoGateway`（无模型时回落） |
| `log.rs` | `Log`：`logs/` 下按时间戳一份文件 |

### 3.3 `presentation/`（呈现）

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 呈现层出口 |
| `cli.rs` | 终端转录中心：解析命令 → 用能力面 → 渲染事件流 |
| `web.rs` | Web 转录中心：tiny_http + 长轮询增量推送（只绑 `127.0.0.1`），分发由路由目录驱动 |
| `intent.rs` | **共享意图层**：CLI 与 Web 的「意图 → 能力调用」规则只此一份（点名、归并、唯一名、动作分发） |
| `routes.rs` | **HTTP 入站契约的唯一定义**：`ROUTES` 目录 + 匹配器（下文 §四 的表与它机器比对） |
| `web/` | 浏览器端：`app.js` / `md.js` / `style.css` / `index.html`，以及 `*.smoke.cjs` 冒烟 |

## 四、呈现层入站契约（命令/事件 + 路由目录）

**核心常驻自己的执行线程、独占全部状态**：呈现层拿不到 `Core`、也拿不到任何核心锁。两侧只通过两样东西来往：

| 东西 | 定义在 | 形态 |
| --- | --- | --- |
| 能力接口 | `core/api.rs` | `SessionOps` / `RegistryOps` / `HistoryOps` / `DiscoveryOps`（全 `&self`，可替换成假实现） |
| 事件台 | `core/api.rs` | `EventBus`：核心独占生产，任意数量的消费者按序号增量取 |

规则：

- **命令**：呈现层调能力接口 → 核心在自己的线程上执行 → 同步回包（`Advance`：本批事件 + 事件台序号）。
- **事件**：生成过程中的短暂事件与最终事件都进事件台；Web 长轮询按 `since` 取，客户端按 `seq` 去重。
- **并发归核心**：`stop` 直接置位核心内部的取消标志，**不进命令队列**，所以生成期间照样立刻生效；
  呈现层不需要知道「生成时核心状态被占用」这类内部事实。
- **一次命令 panic 不带垮核心**：接住并继续服务（回包通道断开，调用方得到「无回应」）。
- **传输的线格式归呈现层**：请求形状、路由、错误码在 `routes.rs`；**事实**的线格式（`SessionEvent`、`*View`）
  仍在 core。两者不混——混在一起就是「到处内联 JSON 拼装」的成因。

### 4.1 HTTP 路由目录（机器可读）

`presentation/routes.rs` 的 `ROUTES` 是路由的**唯一定义**：`web.rs` 的匹配与分发都由它驱动
（匹配由目录做、分支按 `id`），所以「代码里有路由但目录里没有」在结构上不可能发生。
`solomni --print-routes` 输出它的 JSON（含能力、请求/响应形状、状态码与说明）。

下面这张表由契约测试与 `ROUTES` 机器比对——对不上就是测试失败，不是靠人记得改文档：

<!-- ROUTES:BEGIN -->
| 方法 | 路径 | 能力 | 请求 | 响应 | 状态码 |
| --- | --- | --- | --- | --- | --- |
| GET | `/` | 静态资源 | — | `index.html` | 200 |
| GET | `/style.css` | 静态资源 | — | `style.css` | 200 |
| GET | `/app.js` | 静态资源 | — | `app.js` | 200 |
| GET | `/md.js` | 静态资源 | — | `md.js` | 200 |
| GET | `/api/events` | 事件台（`EventBus`） | 查询 `sid` / `since` | `{lines:[{seq,sid,events}],head}` | 200 |
| GET | `/api/state` | `DiscoveryOps` + `RegistryOps` + `HistoryOps` | — | `{modules,rejected,fence,providers,models,core,agents,settings,sessions,history}` | 200, 400 |
| POST | `/api/sessions` | `SessionOps::create_work` | `{name,mode,agents[],task?,delegate?}` | `{sid,agents,events}` | 200, 400 |
| POST | `/api/sessions/{sid}/{action}` | `SessionOps` + `intent::act` | `{text?,agent?,id?,overwrite?,data_base64?,编辑体}` | `{sid,events,seq}` 等 | 200, 400, 404, 409 |
| GET | `/api/sessions/{sid}/config` | `SessionOps::config` | — | `{config}` | 200, 400 |
| GET | `/api/sessions/{sid}/files` | `SessionOps::files` | — | `{work,agents,roots}` | 200, 404 |
| POST | `/api/providers` | `RegistryOps::upsert_provider` | `{id,base_url,api_key}` | `{ok}` | 200, 400 |
| POST | `/api/providers/{id}/{action}` | `RegistryOps::remove_provider` / `discover_models` | — | `{ok}` / `{ok,models}` | 200, 400, 404 |
| POST | `/api/models` | `RegistryOps::upsert_model` | `{id,name,api_model,provider,note?}` | `{ok}` | 200, 400 |
| POST | `/api/models/{id}/{action}` | `RegistryOps::remove_model` / `set_core_model` | — | `{ok}` | 200, 400, 404 |
| POST | `/api/agents` | `RegistryOps::upsert_agent` | `{name,modules[],model?,note?}` | `{ok}` | 200, 400 |
| POST | `/api/agents/{name}/{action}` | `RegistryOps::remove_agent` | — | `{ok}` | 200, 400, 404 |
| GET | `/api/settings` | `RegistryOps::settings` | — | `{settings}` | 200, 400 |
| POST | `/api/settings` | `RegistryOps::set_settings` | `{streaming?,show_reasoning?}` | `{ok}` | 200, 400 |
| GET | `/api/history` | `HistoryOps::list` | — | `{sessions}` | 200, 400 |
| GET | `/api/history/{name}` | `HistoryOps::open` | — | `{meta,events}` | 200, 404 |
| POST | `/api/history/{name}/delete` | `HistoryOps::delete` | — | `{ok}` | 200, 400 |
| POST | `/api/suggest-models` | `DiscoveryOps::suggest_models` | `{task,mode}` | `{ok,agents}` | 200, 400 |
<!-- ROUTES:END -->

## 五、运行日志（Log 端口）

- core 定义 `Log`（`info`/`warn`/`error`），**只调用**；文件、时间戳、目录机制在 adapters。
- 关键节点必须埋点：通道降级、HTTP 失败、会话动作失败、装配失败、工具执行异常。
- 适配层实现（`adapters/log.rs`）：每次运行在根目录 `logs/` 下按时间戳建一个 `.log` 文件；`logs/` 不入库。
- 组合根创建唯一的 `FileLog` 并注入 core 与呈现层；测试用 `NoopLog`。
- 目的：出问题时**看日志定因**，不靠推理猜。

## 六、提示词册（prompts.yaml）

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
| | `sys_tools` | 内置工具说明块（含本 agent 的真实根目录、模块目录与 `{{tool_params}}` 参数签名） |
| | `builtin_tools` | 内置三件套的**参数契约**：模型侧说明与调用校验的唯一来源（不写进代码） |
| | `no_agents` / `no_model` / `no_module_dirs` / `no_module_tools` / `no_module_tool_params` | 空态说法 |
| | `module_tool_params_header` | 模块工具参数段的小标题（模块在 `module.yaml` 里声明了 `params` 时出现） |
| | `tool_texts.*` | **运行时回执**：路径校验、参数不符（说事实 + 回发工具签名）、内置三件套回执与行区间/截断/编码标注、外部工具分派的三类失败、工具超限、**信封不合法四类**（未闭合 / 裸控制字符 / 语法错 / 字段不合法）、给模型看的清单骨架、追加在回复行末尾的 `（已停止）` |

- **界面通知**（`[建组]`、`[上限]` 这类）是呈现层文案，**不属于**提示词册。
- **不进册子的两类**（有意留在代码里）：①**会被解析的转录锚点**（`[轮次 N]`、`[用户:需求]`、`[代拟] …`、`[id:tag]` 等，`collab_state` 与回档定位要读它们，改文案等于改状态机）；②**只给用户看的呈现层文案**（各类 `SessionEvent::Notice`、工具轨迹行的成败字样、面向界面/CLI 的 `Err`）。
- 运行时回执之所以进册子：它们会成为模型下一轮的输入，属于提示词。

## 七、状态与落盘契约

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
- **行上的判定走结构化字段**：例如「信封缺失、按发言原文收录」的降级行带 `degraded: true`，呈现层据此做样式——**不匹配行文本里的说明文案**（改文案不得影响行为）。列的语义同理（`tool` 视图、稳定 `id`）都挂在字段上。
- `meta.yaml` 的 `agents` 是名单的**唯一真相**（代拟路径在用户确认名单那一刻写回）。
- `meta.yaml` 的 `exec` 段是**执行选型**的唯一真相：档位（`tier` = 本机 / 虚拟机）、虚拟机基础根、能力定版（`pins`）、是否放行出站网络；
  缺这段的 `meta.yaml` 按默认读回（本机档、不定版、不联网）。执行计划本身（`core/exec.rs` 的 `ExecPlan`）**从不落盘**——它含真实路径，只在运行时派生。
- 会话的**旁路配置记录**（`{"type":"config"}`）只在编辑提交时追加：供呈现与审计，**不进模型上下文**，回放与状态派生都跳过它。
- `core/fence.rs` 是工具进程围栏的**策略**（可达范围 = 共享区 + 自己的私有沙箱 + 自己的模块目录、断网、工作目录），
  机制在 `adapters/confine/`：外层拉起的**守门进程**（本程序 `--fence-run` 模式）按平台把围栏装进真正的工具进程
  ——Linux Landlock、macOS seatbelt、Windows AppContainer（先建容器 profile，再按 agent 派生容器 SID 与目录 ACL 授权，
  不给 capability 即断网）+ Job Object（进程树）；Windows 的目录授权由外层进程一次性做好（`confine::prepare_fence`）并记在内存台账里。
  装不上就**如实降级**（启动时自检并报告能力等级，绝不假装有）。命令行是守门进程的内部协议，模块作者与用户都不接触。
- `core/packages.rs` 是运行包契约与包库事实（校验、去重、系统路径冲突预检、能力索引），
  `core/exec.rs` 是执行档位与执行计划派生；两者都是纯逻辑，目录遍历在 `PackageSource` 适配层。契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)。
- 登记处四份 yaml 的字段与读写规则见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)。
- 内存与落盘不一致时**以流水为准**（可回放、可重建）。

## 八、可测性（架构约束）

测试的层级、替身语义、端口契约矩阵、质量门禁、缺口账与执行入口全部由 [TESTING.md](TESTING.md) 规定；
本节只列架构对可测性的硬约束，不重复测试规范。

- 任意需要 IO 或存在可替换实现的机制必须通过 `core/ports.rs` 中的端口注入；core 不直接依赖真实模型、网络、文件系统、时钟、随机数或外部进程。
- 纯逻辑（信封解析、协作状态派生、提示词渲染、路径寻址等）不为测试强行增加 trait，直接以纯函数测试；端口只放在真实边界和确有替换价值的点上。
- 端口的输入、输出、错误、取消、超时、重复调用和资源清理语义属于架构契约：生产适配器与测试替身必须遵守同一份契约。
- 端口不能为了方便测试暴露生产实现的内部状态；需要观察交互时，通过测试替身的记录能力或公开的行为结果观察。
- 组合根测试当前使用的内存装配（`InMemory*`、`VecSource`、`ScriptGateway`、`NoopLog` 等）集中在 `src/tests.rs`；
  新增替身用能表达职责的名称，并在测试基础设施中集中维护。
- 质量门禁（格式、编译、Clippy、依赖重复、测试结构冗余）与业务测试是两类事实，分别记录，质量失败不能被业务测试通过抵消。
- 代码冗余检查不改变分层与端口设计，也不以增加 trait、包装层或测试用例为目标；发现重复时先判断是否同一职责，再决定合并、保留或记录原因。

## 九、跨平台机制

- 路径一律用 `PathBuf`/`Path` 组件拼接：**禁止把 `/` 或 `\` 写进字符串再拼**（分隔符交给运行环境）。
- **对外**（提示词、工具参数、回执、API）一律用 `/` 书写形式：Windows 的反斜杠在 JSON 字符串里是**非法转义**（`\A`、`\S` 之类），模型据此拼出的参数会直接解析失败。
- 编码：读严格 UTF-8、非法字节如实标注（**不猜编码**）；写一律 UTF-8；为工具子进程强制 UTF-8 环境。
- 不假设平台：不写死盘符、不假设 shell（工具命令由模块作者声明）。
- 围栏按平台给不同机制（`adapters/confine/` 一个平台一个文件），能力等级如实上报；某平台没有接入的部分**就是没有**——文档与界面都照实说，不用夸张的措辞补齐。
