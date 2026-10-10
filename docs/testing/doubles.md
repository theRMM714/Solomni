# 测试替身与 Fake 专项验收

> 本文是**替身语义的唯一权威**：[ARCHITECTURE.md](../../ARCHITECTURE.md) 只规定"端口必须可注入"，
> [MODULE_SPEC.md](../../MODULE_SPEC.md) 只规定"模块作者要交付什么"，两者都不复述替身语义。
> 层级与判定见 [levels.md](levels.md)；**端口 × 替身 × 真实适配器**的矩阵就在本文 §三。

## 一、测试替身规范

### 1.1 Stub

Stub 只提供预设输入或结果，不负责验证交互。例如固定的设置、能力报告或时间来源。测试需要验证调用次数或顺序时，不能只用 Stub。

### 1.2 Fake

Fake 是可运行但简化的端口实现。它应当让协调业务在没有真实网络、文件系统或外部服务时运行真实业务流程。

Fake 必须：

- 实现明确的端口；
- 支持成功、失败、空结果和边界输入；
- 在端口有此语义时支持延迟、取消、超时或断开；
- 记录被测代码需要观察的调用现场；
- 通过最小端口契约测试；
- 不得只有"永远成功"的 happy path；
- 不得偷偷改变生产端口的错误、顺序或资源语义。

当前项目中的 Fake 或 Fake 候选：

| 实现 | 当前角色 | 当前状态 |
| --- | --- | --- |
| `src/capabilities/llm/detail/fake_chat.rs:FakeChat` | 脚本模型，同时记录 `calls`，兼具 Fake + Spy | 契约已就位（成功 / 空 / 流式 / 中止 / 记录） |
| `src/capabilities/llm/detail/fake_chat.rs:DemoGateway` | 演示/回落网关 | 契约已就位（两类通道 / 回落告知 / 无网络无密钥） |
| `src/tests/doubles.rs:InMemorySettings`、`InMemoryHistory`、`InMemoryWorkspace`、`InMemorySysIo`、`InMemoryWorkStore` | 内存 Fake | 已被核心测试装配使用；端口矩阵已登记（见 §三，已验收） |
| `src/tests/doubles.rs:InMemoryPackages` | 包库 Fake | 已被核心测试使用；端口矩阵已登记（见 §三，已验收） |
| `src/tests/doubles.rs:FakeCatalog` | 模型目录 Fake + 调用记录（`seen`） | 已被核心测试使用；端口矩阵已登记（见 §三，已验收） |
| `src/tests/doubles.rs:VecSource` | 模块清单 Fake | 已被核心测试使用；端口矩阵已登记（见 §三，已验收） |
| `src/tests/doubles.rs:ScriptGateway`、`SharedScript` | 脚本网关 Fake | 已被核心测试使用；端口矩阵已登记（失败注入：无通道回落如实告知） |
| `src/tests/doubles.rs:TestPrompts` | 提示词册 Fake（返回内存册子） | 已被核心测试使用；端口矩阵已登记（见 §三，已验收） |
| `src/tests/builders.rs:RecordingRunner` | 工具执行 Fake + 记录 `calls` | 已被核心测试使用；端口矩阵已登记（失败注入：`ok=false` 回执；超时杀树在 `ProcTools`） |
| `src/tests/builders.rs:ParallelRunner` | 工具执行 Spy：记录**同时在跑**的峰值 | 已钉住"声明可并发才并发、未声明一律串行" |
| `src/tests/builders.rs:NativeGateway`、`NativeChat` | 原生通道替身：按脚本发结构化调用，并记录每次请求的声明与消息 | 已钉住协议形状与"实时/重建逐条一致" |
| `src/tests/builders.rs:SilentRunner` | 守护 Stub：任何调用即 panic | 用于"不该用工具"的路径 |
| `src/tests/doubles.rs:NoFenceHost` | 围栏释放空操作 Stub | 已被核心测试使用 |
| `src/tests/doubles.rs:RecordingFence` | 围栏释放记录型 Spy | 已钉住"删会话即请求撤销授权" |
| `src/kernel/ports.rs:NoopLog` | 无声日志 Stub | 已存在；不用于验证日志内容 |
| `src/tests/doubles.rs:FixedProbe` | 宿主探测 Fake（按声明回答路径 / 可执行文件 / 虚拟化） | 已用于 T2 契约用例；Core 级装配用真实 `HostProbeAdapter`（要 scratch 目录的真实存在性） |
| `src/tests/doubles.rs:NoopLogOps` | 日志能力 Stub（入站能力面） | 已用于路由契约测试（呈现层不需要它做判定） |
| `tests/cross-platform/e2e/mock.js` | 本地假供应商服务 | 已用于 T5；应覆盖协议错误、断开、延迟等场景 |

### 1.3 Mock

Mock 表达预先声明的交互期望，适用于"必须调用一次""必须先调用 A 再调用 B""失败后禁止继续调用"等契约。

本项目不要求引入第三方 mocking 框架。优先使用手写记录型 Fake/Spy，以减少依赖和跨平台不确定性。只有当交互期望本身是被测行为时，才使用 Mock 语义。

### 1.4 Spy

Spy 记录调用现场供断言。`FakeChat.calls`、`FakeCatalog.seen`、`RecordingRunner.calls`、`RecordingFence.released` 是当前明确的 Spy 记录。Spy 不应改变被测依赖的其他行为，也不能因为记录方便而泄漏生产内部状态。

### 1.5 Fixture

Fixture 是可复用的固定输入或预期输出，例如 provider 配置、模型响应、transcript、工作区文件和工具输出。

Fixture 必须：

- 使用相对路径或测试隔离根；
- 不含真实密钥、账号、机器路径；
- 命名表达场景；
- 避免在多个测试中复制粘贴同一大段文本；
- 在测试失败时能定位到输入来源。

## 二、Fake 专项验收

以下条目当前不是"全部已完成"的声明；未完成项进入 `tests/gaps.yaml`。

### `FakeChat`

至少需要覆盖：

- 空脚本；
- 单条脚本和多条脚本；
- 多次调用时的消耗与重复语义；
- 完整消息列表记录；
- 消息顺序保持；
- streaming 回调行为；
- 回调返回 `false` 时的中止行为；
- 空响应和非法响应交给上层后的处理；
- 调用次数与业务预期一致。

### `DemoGateway`

至少需要覆盖：

- member channel 能被创建并完成调用；
- core channel 能被创建并完成调用；
- 回落通知存在且指向正确模块；
- 演示模式不发网络请求、不需要密钥；
- core 与 member 的脚本语义不会相互污染。

### 本地假供应商

至少需要覆盖：

- 正常响应；
- 非法响应；
- HTTP 错误；
- 延迟；
- 连接断开；
- 流式响应；
- 多次运行隔离；
- 端口、子进程和临时目录清理。

## 三、端口测试矩阵

| 端口 | 当前/计划替身 | 交互记录 | 失败注入 | 取消/超时 | 真实适配器 | 当前状态 |
| --- | --- | --- | --- | --- | --- | --- |
| `Chat` | `FakeChat`、`SharedScript`、`TruncChat`、`AbortChat` | `FakeChat.calls` | 脚本回放非法信封 | `on` 返回 false 中止（FakeChat / HttpChat） | `HttpChat`：结束原因（非流式 `stop` / 流式 `length`）、原生 `tool_calls`（非流式 + 流式按 index 拼分片）都在环回假供应商上验 | 已验收 |
| `ChatGateway` | `ScriptGateway`、`DemoGateway`、`ProbeGateway` | 通道脚本可观察 | 无通道回落（如实告知） | 不适用 | `HttpGateway`：探测的三种结论（支持 / 明确不支持 / 无法判定）与"通道本身不通"都在环回假供应商上验；结论写回登记处只写确凿的；回放形状探测逐项验"收了没有 / 真的读懂没有"（替身没真实供应商时默认如实说测不了） | 已验收 |
| `SettingsStore` | `InMemorySettings` | 内存状态可观察 | `fail_with` | 不适用 | `YamlSettingsStore` | 已验收 |
| `ModelCatalog` | `FakeCatalog` | `seen` | `fail_with` | 不适用 | `HttpModelCatalog` | 已验收 |
| `ModuleSource` | `VecSource` | 不适用 | 不适用（错误进 `rejected`） | 不适用 | `FsModules` | 已验收 |
| `PackageSource` | `InMemoryPackages` | 不适用 | 不适用（错误进 `rejected`） | 不适用 | `FsPackages` | 已验收 |
| `Workdirs` | `InMemoryWorkspace` | 内存布局可观察 | `fail_with` | 不适用 | `FsWorkspace` | 已验收 |
| `WorkStore` | `InMemoryWorkStore` | 内存文件 / 提交 / 基线可观察 | `fail_with` | 不适用 | `FsWorkStore`（含符号链接逃逸拒绝） | 已验收；**持有者只有 `workspace::service.rs`**（R12） |
| `ModuleSource` / `PackageSource` / `Workdirs` / `WorkStore` 的持有者 | ——（四个端口**只由 `workspace::service.rs` 持有**；别的能力经 `workspace::api::Workspace` 要清单、目录与版本库用例） | 不适用 | 不适用 | 不适用 | 不适用 | 已收口 |
| `SysIo` | `InMemorySysIo`（含并发峰值与按文件延时） | 内存内容 + 同时在读的峰值 | `fail_with` | 不适用 | `FsSysIo`（含 lossy / cut） | 已验收；**持有者只有 `tools::service.rs`**（R12） |
| `HistoryStore` | `InMemoryHistory` | 内存流水可观察 | `fail_with` | 不适用 | `FsHistory` | 已验收；**持有者只有 `session::service.rs`**（R12） |
| `PromptSource` | `TestPrompts` | 不适用 | `fail_with` | 不适用 | `YamlPrompts` | 已验收 |
| `SystoolsSource` | 不适用（**直接用真实加载器**：读的就是仓库自己的两份 yaml，确定性足够） | 不适用 | 缺文件 / 缺键由真实加载器如实报错（`tests/detail.rs`） | 不适用 | `YamlSystools` | 已验收 |
| `ProcessRunner`（kernel 共享） | `RecordingRunner`、`SilentRunner`、`ParallelRunner`、`AskingRunner`（跑之前先经提问端口问一次） | `calls`（cwd / 命令 / 参数）、并发峰值；`AskingRunner` 另记"问过几次、真跑了几次" | `ok = false` 回执 | 真进程超时杀树（`ProcTools`） | `ProcTools` | 已验收 |
| `AskUser` | `RecordingAsk`（按脚本作答，记下每次问到的选项 id）、`AskingRunner` 内部的同形替身 | 问到的选项 id / 是否停过会话 | 不适用（作答脚本给 `None` = 拒绝 / 没人答） | 阻塞等回答（不设超时；整队作废解开） | 会话侧实现 `conductor::service::SessionAsk`（卡片进那一条队、`answer_card` 作答；空选项集停会话 + 落警告） | 已验收（`src/tests/conductor/ask_user.rs`、`src/tests/permission.rs` 的端到端一条） |
| `EnvelopeRepair` | `NoRepair` | 不适用 | 不适用（修复器遇不确定一律不修） | 不适用 | `UnambiguousRepair`（转义裸控制字符 + 补上缺的收尾括号；断在字符串中间、起了两段信封一律不修） | 已验收 |
| `FenceHost`（kernel 共享） | `RecordingFence`、`NoFenceHost` | `released` | `fail_with` | 不适用 | `confine::FenceHostAdapter`（真机撤权在 `tests/windows/`） | 已验收 |
| `Log` | `NoopLog` | 不记录（Stub） | 不适用 | 不适用 | `FileLog`（三个级别都落盘） | 已验收 |
| `HostProbe` | `FixedProbe`（只按声明回答） | 不适用 | 不适用 | 不适用 | `HostProbeAdapter`（真实路径事实；PATH 上不存在的名字如实说没有） | 已验收 |

`Log`、`HostProbe`、`ToolHandler`、`AskUser`、`ProcessRunner` 与 `FenceHost` 是**不在某个能力 `ports.rs`** 的端口：它们在 `kernel/ports.rs`（机制型内核，无领域语义，R12 的例外）。
见 [../kernel/unit-map.md](../kernel/unit-map.md)。

"已验收"指该端口在 `src/tests/`（`detail.rs` 覆盖真实实现）的契约测试里有成功、失败、空/边界与交互记录的断言；


