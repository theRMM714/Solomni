# llm（模型通道与协议）

> 定位：建通道、跑一次模型会话、信封解析与补救。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/business/llm/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：通道与协议词汇（`Chat` / `Msg` / `Role` / `Completion` / `Chunk` / `ToolCall` / `ToolDecl` / `Channel`；`ToolMode` 是登记事实，在 `registry_model`）、按已解析通道建会话、把登记处解析好的思考片段并入请求体、实测一条通道支不支持原生工具调用、实测回放形状、模型发现、信封解析与不合法的补救、出站 HTTP 代理与端点记忆。
- 管：**供应商协议**（`Protocol` 端口）：请求体怎么拼、回执怎么读、流式分块怎么解析、用量怎么归一、回放历史长什么样；实现只在 `src/units/business/llm/detail/protocol/`。
- 不管：**不选择用哪个模型**（「用哪个」是登记处解析链里的策略）。
- 不管：不落盘、不管会话。

## 入站契约与状态归属

- 入站面：`api::Llm`（造通道 / 探测 / 发现 / 修信封）+ 对外协议类型（`Chat` 的调用方是别的单元，所以契约在 `api`）。
- 出站端口：`ChatGateway` / `ModelCatalog` / `EnvelopeRepair` / `Protocol` **只由 `src/units/business/llm/service.rs` 与 `detail` 持有**（R12），**不进 `api`**。
- 协议边界：`api` 的 `Role` / `Msg` / `ToolCall` 是**协议无关**形状（`Role` 只有 system / user / assistant / tool 四种）；字段名、请求体形状、`usage` 的两种口径都在 `Protocol` 的实现里。`HttpChat` / `http_stream` / `http_probe` 只管出站机制（端点候选、超时、取消、重试、SSE 分行），不认识字段名。新增一种协议 = 新增一个实现 + `src/units/business/llm/detail/protocol.rs` 一处选择。
- 状态归属：不落盘、不管会话；通道实例与端点记忆由本单元在内存持有，会话状态归 `session`。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`
- 谁在用我：`agentloop`、`catalog`、`collab`、`conversation`、`presentation`、`runtime`、`tools`、`transcript`、`turn`、`turn_state`、`work`

## 一、token 用量与流式回退

- **token 用量是通道事实**：`Completion.usage` 带供应商回的 prompt / completion / total（拿不到 = None），`Chat::last_usage()` 让调用方在一次工具循环结束后读最近一次用量。
- 流式请求带本协议的「流式用量」参数（OpenAI 兼容是 `stream_options.include_usage`）；供应商 400/422 明确拒绝该参数时，`HttpChat` 问 `Protocol::drop_stream_usage` 摘掉它**只重试一次**（此时还没收到内容，不会重复正文），用量退回「拿不到」。
- 非流式直接读响应体 `usage`。
- **两种供应商形状归一**：Anthropic 的 `input_tokens` 只是"没命中缓存的输入"，归一后 prompt = input + cache_read + cache_creation、total = prompt + completion；OpenAI 形状 `prompt_tokens` 已含命中缓存的部分。流式分块只报一部分字段时 `TokenUsage::widen` 按字段取大合并，不拿半份覆盖整份；两种口径的判定与归一都在 `Protocol::event` / `Protocol::reply` 的实现里。

## 二、思考强度片段

- **思考强度片段是请求体的一部分**：登记处按「模型 + 会话档」解析出片段（字符串 = `reasoning_effort` 简写，对象 = 原样合并），`CompleteOpts.thinking` / `LlmOpts.thinking` 一路带到 `Protocol::request`，在**唯一一处**合并；片段里的保留键（`registry_model` 的 `RESERVED_REQUEST_KEYS`）在登记处与适配器各挡一次。
- 响应侧的 `reasoning_content` / `reasoning` 解析与显示不变。

## 改动本单元时必须同步

- 请求体形状是**真实会话与探针共用的唯一定义**（`src/units/business/llm/detail/protocol/openai.rs`，`Protocol::request` / `Protocol::request_raw`）；改协议 → `src/tests/llm.rs`、`src/tests/detail/http.rs`、`src/tests/fakes.rs`；信封不合法的修法由 `src/units/business/llm/domain/malformed.rs` 代码返回。思考片段也走这一处：改合并规则要同步 `src/tests/detail/http.rs` 的请求体断言与 `REGISTRY_SPEC.md` 的 `thinking` 段。流式请求体的「流式用量」参数属于这条协议形状；供应商拒收时的回退在 `HttpChat::complete`，由 `src/tests/detail/http.rs` 钉住「只重试一次、第二次不含该参数」。
- 流式读、停止与分层超时在 `src/units/business/llm/detail/http_stream.rs`；改这里要同步 `src/tests/detail/http.rs` 的静默 / keepalive / 取消用例，以及 `docs/session/session-model.md` 的停止与超时语义。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
