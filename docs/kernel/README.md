# kernel（机制型业务）

> 没有领域语义的机制；依赖图最底层。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：运行日志端口、宿主探测端口、**提问端口**（请用户裁决）、生成中作业的取消表、跨业务共享的**事实类型**、路径的对外书写形式。

**不管**：**不认识任何能力 / 呈现层 / 入口层**；不放有领域语义的类型——它不需要知道什么是回合、回复、工具执行。

## 二、入站契约与状态归属

`api`（`SessionId` / `Tier` / `ToolOutcome` / `Ask` / `DEFAULT_LLM_TIMEOUT_SECS` / `slash` / `JobRegistry`）、
`ports`（`Log`、`HostProbe`、`ToolHandler`、`AskUser`——R12 的例外：全项目共享）、`domain`（事实类型、路径书写、取消表实现）、
`detail`（`FileLog`、`HostProbe`）。

`AskUser` 是「需要用户裁决的机制请用户裁决」的**唯一**入口（形状：`ask(问题) -> 选项 id`，**阻塞**；
`halt(为什么)` = 构不出可用选项时停会话 + 落警告）：工具执行层与围栏用它，实现在会话侧（`conductor` 的 `SessionAsk`），
走的是会话的**统一裁决通道**——见 [docs/session/session-model.md](../session/session-model.md) 的「请用户裁决」。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——依赖图最底层或纯领域）
- 谁在用我：`conductor`、`llm`、`prompt`、`registry`、`session`、`tools`、`workspace`

## 四、改动本单元时必须同步

- 共享事实类型（R6）一改，所有使用者同改；路径拼接规则见 `ARCHITECTURE.md` §八（禁止把分隔符写进字符串）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |