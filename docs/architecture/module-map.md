# 模块地图

> 本文是**模块地图的唯一权威**：`core/`、`adapters/`、`presentation/` 各文件职责一览。
> 分层规则与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md)，呈现层入站契约见 [contracts.md](contracts.md)。

## 一、`core/`（抽象与业务，无 IO）

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 核心层入口与 `Core` 门面：会话中心、登记处编排、运行包报告 |
| `ports.rs` | 出站端口 trait 与跨层数据结构（依赖倒置的边界；core 需要什么，由适配器实现） |
| `api.rs` | **入站契约**：四个按角色的能力接口 + `CoreHandle`（核心自有线程、命令/事件）+ `EventBus` + `JobRegistry`；单 agent 与协作长步骤的生成都在**工作线程**上跑（队列只占"取/交"两步） |
| `events.rs` | 呈现侧契约：`SessionEvent` 与介入请求的词汇（**事实**的线格式定义在这）；转录行 `LineView` 带 `speaker` / `verb` / `kind`，`render()` 是"字段 → 文本"的唯一拼法 |
| `prompt.rs` | 提示词渲染：`{{key}}` 占位替换，缺键/缺变量报错 |
| `schema.rs` | 工具参数契约（**声明在文本层**）：解析/校验/两种渲染（模型侧说明、JSON Schema） |
| `module.rs` | `module.yaml` 契约、扫描结果 `Roster`、`runtimes`/`tools` 校验、agent system 合成 |
| `packages.rs` | `package.yaml` 契约与包库事实（校验、去重、系统路径冲突预检） |
| `exec.rs` | 执行档位（`ExecSpec`）与执行计划（`ExecPlan`）派生、虚拟机档诊断、档位承载（`TierReadiness`：本机能不能承载这个档位） |
| `fence.rs` | 一次工具执行的围栏策略（纯数据：可达根、断网、工作目录） |
| `workspace.rs` | 工作区与沙箱的纯数据定义、寻址与越界判定 |
| `systool.rs` | 内置工具 `read` / `write` / `edit` / `patch` / `search` 的放行、寻址、**按声明校验参数**、改动前的"读过"证据（`Observations`）、自由格式补丁的原子应用与回执文案 |
| `patch.rs` | 补丁通道的**纯逻辑**：解析自由格式补丁（Add / Update / SEARCH / REPLACE / End File）与整行应用（逐行匹配、行尾风格保持、失败点名第几处） |
| `refs.rs` | 用户 `@` 引用改写成真实绝对路径 |
| `providers.rs` | 供应商/模型登记处内存形态与「模型 → 通道」解析 |
| `agents.rs` | agent 登记处、代拟名单落地与名字校验 |
| `history.rs` | 会话元信息与历史视图的内存形态 |
| `envelope.rs` | 发言信封解析（`ToolInvoke.body` = 信封之后的正文，自由格式工具的输入从这里取；含「像工具信封但不合法」的独立信号，并判定**未闭合 / 裸控制字符 / 语法错 / 字段不合法**四类；未闭合带上 EOF 状态：还差哪些收尾字符、是否断在字符串中间、这一段里起了几段信封） |
| `collab_state.rs` | 「转录即状态」的协作状态派生（纯函数、可回放） |
| `collab.rs` | 协作会话状态机与讨论泵（泵只决定"该问谁"、核心驱动成员回合；发言投影、待裁决与工具面发放） |
| `engine.rs` | 讨论/执行/验收的引擎；**唯一的轮循环** `converse_with`（单 agent / 节点 / 讨论席共用：表态与工具两套形态、按声明调度的并发、逐轮外送）；**唯一的请求装配点** `assemble`（身份 + 本回合工具面 + 对话 + 本回合提示） |
| `session.rs` | 单 agent 会话：**会话参数**（`SessionParams`：身份与环境，每次调用现渲染）与**对话**（`dialogue`：只有发生过的事）分开；转录行带稳定 id；`TurnRun` + `build_round_lines`（**唯一的行构造点**）；`discussion_turn` = 讨论席那一回合（同一条循环 + 表态 + 逐轮落进它自己的会话） |

## 二、`adapters/`（机制，实现 core 端口）

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 适配层出口与统一 re-export |
| `confine/` | 守门进程与平台围栏后端：`mod.rs` 装配与能力自报，`linux.rs` / `macos.rs` / `windows.rs` / `other.rs` 各平台机制 |
| `proc_tools.rs` | `ToolRunner`：守门进程拉起、stdin 送参、超时杀树、输出截断 |
| `sys_io.rs` | `SysIo`：内置工具的读写机制（UTF-8 解码、非法字节 `lossy` 标注） |
| `repair.rs` | `EnvelopeRepair`：信封修复（默认只做两类可判定的修补——转义裸控制字符、补上缺的收尾括号；断在字符串中间与其余类别一律不猜） |
| `fs_modules.rs` | `ModuleSource`：扫描 `modules/` |
| `fs_packages.rs` | `PackageSource`：扫描 `runtimes/` |
| `fs_workspace.rs` | `Workspace`：`session/<工作名>/` 下的 work 与各 agent 沙箱 |
| `fs_history.rs` | `HistoryStore`：`meta.yaml` + `transcript.jsonl` |
| `yaml_settings.rs` | `SettingsStore`：登记处四份 yaml 的读写（见 [REGISTRY_SPEC.md](../../REGISTRY_SPEC.md)） |
| `yaml_prompts.rs` | `PromptSource`：加载 `prompts/` |
| `endpoint.rs` | 端点补全/回落规则与进程内端点记忆（纯逻辑） |
| `http_agent.rs` | 出站 HTTP 代理构建（TLS 后端选择与超时的单点） |
| `http_chat.rs` | `Chat` / `ChatGateway`：OpenAI 兼容 `/chat/completions`（请求体形状的唯一定义：真实会话与探针共用） |
| `http_probe.rs` | 两条诊断探针（只报事实）：工具调用支持探测（`--probe-tools`）、回放形状探测（`--probe-replay`） |
| `model_catalog.rs` | `ModelCatalog`：OpenAI 兼容 `GET /models` |
| `fake_chat.rs` | 演示/测试通道：`FakeChat`（脚本回放）+ `DemoGateway`（无模型时回落） |
| `log.rs` | `Log`：`logs/` 下按时间戳一份文件 |

## 三、`presentation/`（呈现）

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 呈现层出口 |
| `cli.rs` | 终端转录中心：解析命令 → 用能力面 → 渲染事件流 |
| `web.rs` | Web 转录中心：tiny_http + 长轮询增量推送（只绑 `127.0.0.1`），分发由路由目录驱动 |
| `intent.rs` | **共享意图层**：CLI 与 Web 的「意图 → 能力调用」规则只此一份（点名、归并、唯一名、动作分发） |
| `routes.rs` | **HTTP 入站契约的唯一定义**：`ROUTES` 目录 + 匹配器（下文 §四 的表与它机器比对） |
| `web/` | 浏览器端：`app.js` / `md.js` / `style.css` / `index.html`，以及 `*.smoke.cjs` 冒烟 |

