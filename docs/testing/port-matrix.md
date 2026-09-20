# 端口测试矩阵

> 端口矩阵是测试设计账：不允许只写"有 mock"而不说明 Fake 的能力。
> 替身语义见 [doubles.md](doubles.md)，层级与判定见 [levels.md](levels.md)。

## 六、端口测试矩阵

端口矩阵是测试设计账，不允许只写"有 mock"而不说明 Fake 的能力。

| 端口 | 当前/计划替身 | 交互记录 | 失败注入 | 取消/超时 | 真实适配器 | 当前状态 |
| --- | --- | --- | --- | --- | --- | --- |
| `Chat` | `FakeChat`、`SharedScript`、`TruncChat`、`AbortChat` | `FakeChat.calls` | 脚本回放非法信封 | `on` 返回 false 中止（FakeChat / HttpChat） | `HttpChat`：结束原因（非流式 `stop` / 流式 `length`）、原生 `tool_calls`（非流式 + 流式按 index 拼分片）都在环回假供应商上验 | 已验收 |
| `ChatGateway` | `ScriptGateway`、`DemoGateway`、`ProbeGateway` | 通道脚本可观察 | 无通道回落（如实告知） | 不适用 | `HttpGateway`：探测的三种结论（支持 / 明确不支持 / 无法判定）与"通道本身不通"都在环回假供应商上验；结论写回登记处只写确凿的；回放形状探测逐项验"收了没有 / 真的读懂没有"（替身没真实供应商时默认如实说测不了） | 已验收 |
| `SettingsStore` | `InMemorySettings` | 内存状态可观察 | `fail_with` | 不适用 | `YamlSettingsStore` | 已验收 |
| `ModelCatalog` | `FakeCatalog` | `seen` | `fail_with` | 不适用 | `HttpModelCatalog` | 已验收 |
| `ModuleSource` | `VecSource` | 不适用 | 不适用（错误进 `rejected`） | 不适用 | `FsModules` | 已验收 |
| `PackageSource` | `InMemoryPackages` | 不适用 | 不适用（错误进 `rejected`） | 不适用 | `FsPackages` | 已验收 |
| `Workspace` | `InMemoryWorkspace` | 内存布局可观察 | `fail_with` | 不适用 | `FsWorkspace` | 已验收 |
| `SysIo` | `InMemorySysIo`（含并发峰值与按文件延时） | 内存内容 + 同时在读的峰值 | `fail_with` | 不适用 | `FsSysIo`（含 lossy / cut） | 已验收 |
| `HistoryStore` | `InMemoryHistory` | 内存流水可观察 | `fail_with` | 不适用 | `FsHistory` | 已验收 |
| `PromptSource` | `TestPrompts` | 不适用 | `fail_with` | 不适用 | `YamlPrompts` | 已验收 |
| `ToolRunner` | `RecordingRunner`、`SilentRunner`、`ParallelRunner` | `calls`（cwd / 命令 / 参数）、并发峰值 | `ok = false` 回执 | 真进程超时杀树（`ProcTools`） | `ProcTools` | 已验收 |
| `FenceHost` | `RecordingFence`、`NoFenceHost` | `released` | `fail_with` | 不适用 | `confine::FenceHostAdapter`（真机撤权在 `tests/windows/`） | 已验收 |
| `Log` | `NoopLog` | 不记录（Stub） | 不适用 | 不适用 | `FileLog`（三个级别都落盘） | 已验收 |

"已验收"指该端口在 `src/contract_tests/` 与 `src/adapters/*` 的契约测试里有成功、失败、空/边界与交互记录的断言；
真实适配器边界的覆盖范围以本表的"真实适配器"列为准。新增端口或新增替身必须同时补齐这一行。

