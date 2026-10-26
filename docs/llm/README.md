# llm（模型通道与协议）

> 建通道、跑一次模型会话、信封解析与补救。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：通道与协议词汇（`Chat` / `Msg` / `Completion` / `Chunk` / `ToolCall` / `ToolDecl` / `Channel` / `ToolMode`）、按已解析通道建会话、实测一条通道支不支持原生工具调用、实测回放形状、模型发现、信封解析与不合法的补救、出站 HTTP 代理与端点记忆。

**不管**：**不选择用哪个模型**（「用哪个」是登记处解析链里的策略）；不落盘、不管会话。

## 二、入站契约与状态归属

`api::Llm`（造通道 / 探测 / 发现 / 修信封）+ 对外协议类型（`Chat` 的调用方是别的能力，所以契约在 `api`）。出站端口 `ChatGateway` / `ModelCatalog` / `EnvelopeRepair` **只由 `service.rs` 持有**（R12），**不进 `api`**。

**token 用量是通道事实**：`Completion.usage` 带供应商回的 prompt / completion / total（拿不到 = None），
`Chat::last_usage()` 让调用方在一次工具循环结束后读最近一次用量。流式请求带
`stream_options: {include_usage: true}`；供应商 400/422 明确拒绝该参数时，`HttpChat` 去掉参数**只重试一次**
（此时还没收到内容，不会重复正文），用量退回"拿不到"。非流式直接读响应体 `usage`。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`prompt`
- 谁在用我：`collab`、`conductor`、`registry`、`session`、`slate`、`tools`

## 四、改动本单元时必须同步

- 请求体形状是**真实会话与探针共用的唯一定义**（`detail/http_chat.rs`）；改协议 → `src/tests/llm.rs`、`src/tests/detail.rs`、`src/tests/fakes.rs`；信封不合法的修法由 `llm/domain/malformed.rs` 代码返回。
  流式请求体的 `stream_options.include_usage` 属于这条协议形状；供应商拒收时的回退在 `HttpChat::complete`，由 `src/tests/detail.rs` 钉住"只重试一次、第二次不含该参数"。
- 流式读、停止与分层超时在 `detail/http_stream.rs`；改这里要同步 `src/tests/detail.rs` 的静默 / keepalive / 取消用例，以及 `docs/session/session-model.md` 的停止与超时语义。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |