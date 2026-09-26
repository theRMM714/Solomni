# 测试替身与 Fake 专项验收

> 本文是**替身语义的唯一权威**：[ARCHITECTURE.md](../../ARCHITECTURE.md) 只规定"端口必须可注入"，
> [MODULE_SPEC.md](../../MODULE_SPEC.md) 只规定"模块作者要交付什么"，两者都不复述替身语义。
> 层级与判定见 [levels.md](levels.md)，端口矩阵见 [port-matrix.md](port-matrix.md)。

## 一、测试替身规范

### 1.1 Stub

Stub 只提供预设输入或结果，不负责验证交互。例如固定的设置、能力报告或时间来源。测试需要验证调用次数或顺序时，不能只用 Stub。

### 1.2 Fake

Fake 是可运行但简化的端口实现。它应当让 core 在没有真实网络、文件系统或外部服务时运行真实业务流程。

Fake 必须：

- 实现明确的 core 端口；
- 支持成功、失败、空结果和边界输入；
- 在端口有此语义时支持延迟、取消、超时或断开；
- 记录被测代码需要观察的调用现场；
- 通过最小端口契约测试；
- 不得只有"永远成功"的 happy path；
- 不得偷偷改变生产端口的错误、顺序或资源语义。

当前项目中的 Fake 或 Fake 候选：

| 实现 | 当前角色 | 当前状态 |
| --- | --- | --- |
| `src/adapters/fake_chat.rs:FakeChat` | 脚本模型，同时记录 `calls`，兼具 Fake + Spy | 契约已就位（成功 / 空 / 流式 / 中止 / 记录） |
| `src/adapters/fake_chat.rs:DemoGateway` | 演示/回落网关 | 契约已就位（两类通道 / 回落告知 / 无网络无密钥） |
| `src/tests/doubles.rs:InMemorySettings`、`InMemoryHistory`、`InMemoryWorkspace`、`InMemorySysIo` | 内存 Fake | 已被核心测试装配使用；端口矩阵已登记（`port-matrix.md`，已验收） |
| `src/tests/doubles.rs:InMemoryPackages` | 包库 Fake | 已被核心测试使用；端口矩阵已登记（`port-matrix.md`，已验收） |
| `src/tests/doubles.rs:FakeCatalog` | 模型目录 Fake + 调用记录（`seen`） | 已被核心测试使用；端口矩阵已登记（`port-matrix.md`，已验收） |
| `src/tests/doubles.rs:VecSource` | 模块清单 Fake | 已被核心测试使用；端口矩阵已登记（`port-matrix.md`，已验收） |
| `src/tests/doubles.rs:ScriptGateway`、`SharedScript` | 脚本网关 Fake | 已被核心测试使用；端口矩阵已登记（失败注入：无通道回落如实告知） |
| `src/tests/doubles.rs:TestPrompts` | 提示词册 Fake（返回内存册子） | 已被核心测试使用；端口矩阵已登记（`port-matrix.md`，已验收） |
| `src/tests/core.rs:RecordingRunner` | 工具执行 Fake + 记录 `calls` | 已被核心测试使用；端口矩阵已登记（失败注入：`ok=false` 回执；超时杀树在 `ProcTools`） |
| `src/tests/core.rs:ParallelRunner` | 工具执行 Spy：记录**同时在跑**的峰值 | 已钉住"声明可并发才并发、未声明一律串行" |
| `src/tests/core.rs:NativeGateway`、`NativeChat` | 原生通道替身：按脚本发结构化调用，并记录每次请求的声明与消息 | 已钉住协议形状与"实时/重建逐条一致" |
| `src/tests/core.rs:SilentRunner` | 守护 Stub：任何调用即 panic | 用于"不该用工具"的路径 |
| `src/tests/doubles.rs:NoFenceHost` | 围栏释放空操作 Stub | 已被核心测试使用 |
| `src/tests/doubles.rs:RecordingFence` | 围栏释放记录型 Spy | 已钉住"删会话即请求撤销授权" |
| `src/core/ports.rs:NoopLog` | 无声日志 Stub | 已存在；不用于验证日志内容 |
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

