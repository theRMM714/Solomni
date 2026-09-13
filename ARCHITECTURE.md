# 架构与开发规则（ARCHITECTURE）

> 改代码前必读。本文是**分层的唯一权威**：谁依赖谁、端口画在哪、机制放哪一层、日志与提示词册怎么用。
> 理念见 [PHILOSOPHY.md](PHILOSOPHY.md)，产品行为见 [PRODUCT.md](PRODUCT.md)，模块作者契约见 [MODULE_SPEC.md](MODULE_SPEC.md)，仓库协作规则见 [AGENTS.md](AGENTS.md)。

## 一、分层与依赖方向

依赖箭头只有一种画法：

```text
presentation ──▶ core ◀── adapters
                   ▲
              main（组合根，装配全部）
```

| 层 | 干什么 | 禁令 |
| --- | --- | --- |
| `core/` | 定义抽象（`ports.rs`）+ 编排业务（会话、协作状态机、引擎、信封解析） | 不读文件（`std::fs`）、不发网络（ureq）、不碰 stdin/stdout——一切机制下沉适配层 |
| `adapters/` | 实现 core 的端口；可引用外部库（ureq / serde_yaml） | 只依赖 core，**永不反向**；不做装配决策 |
| `presentation/` | 渲染事件、收集输入（CLI 与 Web 并列） | 只依赖 core 门面；**永不接触端口对象** |
| `main.rs` | 组合根：`new` 出所有适配器并注入 | 除装配外无业务 |

推论：

- 「用哪个供应商/模型」是**策略**，在 core 的解析链里决定；「怎么建通道」是**机制**，在 adapters。两者不互换。
- 呈现层拿到的是 `Core` 门面 + `SessionEvent` 流；它不知道端口的存在。

## 二、端口：只画在 IO 与可替换点上

端口 = `core/ports.rs` 里的 trait。判据只有一条：**这里有 IO，或者这里有可替换的实现**。
纯逻辑（信封解析、引擎循环、协作状态派生、提示词渲染）**刻意不抽象**——它们没有 trait。

| 端口 | 职责 | 适配层实现 |
| --- | --- | --- |
| `Chat` | 一次模型会话：收消息列表回原文；`on` 逐片回调，返回 `false` 即要求中止 | `HttpChat`（测试 `FakeChat`） |
| `ChatGateway` | 建通道（含核心通道与回落告知）；**不选择**模型 | `HttpGateway` |
| `SettingsStore` | 登记处持久化（providers / models / settings / agents 四个 yaml） | `YamlSettingsStore` |
| `ModelCatalog` | 列出一条通道当前可用的模型名 | `HttpModelCatalog` |
| `ModuleSource` | 模块清单来源（扫描 `modules/`） | `FsModules` |
| `Workspace` | 一次工作的 work 目录、各 agent 沙箱、文件清单与寻址根 | `FsWorkspace` |
| `SysIo` | 内置文件工具的读写机制（读严格 UTF-8、非法字节如实标注；写一律 UTF-8） | `FsSysIo` |
| `HistoryStore` | 会话历史：一个会话一个目录（meta + 事件流水） | `FsHistory` |
| `PromptSource` | 提示词册加载（`prompts.yaml`） | `YamlPrompts` |
| `ToolRunner` | 外部工具进程（拉起、stdin 送参、超时、截断） | `ProcTools` |
| `Log` | 运行日志（三级） | `FileLog`（测试 `NoopLog`） |

新增端口前先问一句：**这是 IO 或可替换点吗**？不是就别加 trait。

## 三、运行日志（Log 端口）

- core 定义 `Log`（`info`/`warn`/`error`），**只调用**；文件、时间戳、目录机制在 adapters。
- 关键节点必须埋点：通道降级、HTTP 失败、会话动作失败、装配失败、工具执行异常。
- 适配层实现（`adapters/log.rs`）：每次运行在根目录 `logs/` 下按时间戳建一个 `.log` 文件；`logs/` 不入库。
- 组合根创建唯一的 `FileLog` 并注入 core 与呈现层；测试用 `NoopLog`。
- 目的：出问题时**看日志定因**，不靠推理猜。

## 四、提示词册（prompts.yaml）

- **所有发给 LLM 的提示词一律写入 `prompts.yaml`**，禁止硬编码进代码；改文案只改册子。
- 占位符 `{{key}}`；渲染器在 `core/prompt.rs`（纯逻辑）；文件加载经 `PromptSource` 端口在适配层。
- **缺文件 / 缺键 / 缺变量 = 报错暴露**，禁止静默兜底文案。
- **界面通知**（`[建组]`、`[上限]` 这类）是呈现层文案，**不属于**提示词册。
- **运行时回执也算提示词**：`core.tool_texts` 段收口了工具与路径的失败/回执文案（路径校验、内置三件套的回执与截断/编码标注、外部工具分派的三类失败、工具超限、信封非法、给模型看的清单骨架、追加在回复行末尾的`（已停止）`）——它们会成为模型下一轮的输入，因此**不硬编码在代码里**。
- **不进册子的两类**（有意留在代码里）：①**会被解析的转录锚点**（`[轮次 N]`、`[用户:需求]`、`[代拟] …`、`[id:tag]` 等，`collab_state` 与回档定位要读它们，改文案等于改状态机）；②**只给用户看的呈现层文案**（各类 `SessionEvent::Notice`、工具轨迹行的成败字样、面向界面/CLI 的 Err）。
- 文案的注入方式与端口一致：随环境对象传入（沙箱/工具环境/引用改写器），而不是让纯逻辑自己去读文件。
- 路径类占位符（`{{work_root}}` 等）由 core 在运行时替换成**真实根目录**后才交给 AI——仓库里永远不出现机器路径。

## 五、状态与落盘契约

**布局**（机制口径）：

```text
session/<工作名>/
  meta.yaml          # 身份与选型：形态、agent 名单、模块、模型、需求
  transcript.jsonl   # 只追加的事件流水
  work/              # 本次工作共享区（用户投喂与成品）
  <agent实例名>/      # 该 agent 的私有沙箱
```

- **转录即状态**：流水只追加；回档**只追加一条 `{"type":"rewind"}` 记录**，不物理删行；会话内容 = 回放到最后一个截断点。
- **转录行的稳定 id**：一轮模型调用 = 一条行；工具调用自成一条行；id 在会话内单调、回放可复现（回档按 id 定位）。
- **流式增量是短暂事件**：`delta` / `tool_call` 不落盘；历史只记定稿后的行。
- `meta.yaml` 的 `agents` 是名单的**唯一真相**（代拟路径在用户确认名单那一刻写回）。
- 内存与落盘不一致时**以流水为准**（可回放、可重建）。

## 六、可测性

- 每个模块可单独 mock 测试；测试里的"组合根"就是内存适配器：`InMemorySettings` / `InMemoryHistory` / `InMemoryWorkspace` / `InMemorySysIo`、`VecSource`、`ScriptGateway`、`NoopLog`。
- core 的可测性来自端口化：断言不需要真实模型、不需要文件系统、不需要网络。
- 纯逻辑（信封、协作状态派生、提示词渲染、路径寻址）都有独立的纯函数测试。

## 七、跨平台机制

- 路径一律用 `PathBuf`/`Path` 组件拼接：**禁止把 `/` 或 `\\` 写进字符串再拼**（分隔符交给运行环境）。
- **对外**（提示词、工具参数、回执、API）一律用 `/` 书写形式：Windows 的反斜杠在 JSON 字符串里是**非法转义**（`\A`、`\S` 之类），模型据此拼出的参数会直接解析失败。
- 编码：读严格 UTF-8、非法字节如实标注（**不猜编码**）；写一律 UTF-8；为工具子进程强制 UTF-8 环境。
- 不假设平台：不写死盘符、不假设 shell（工具命令由模块作者声明）。
