# llm（模型通道与协议）

> 建通道、跑一次模型会话、信封解析与补救。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：通道与协议词汇（`Chat` / `Msg` / `Completion` / `Chunk` / `ToolCall` / `ToolDecl` / `Channel` / `ToolMode`）、建通道与回落、实测一条通道支不支持原生工具调用、模型发现、信封解析与不合法的补救、出站 HTTP 代理与端点记忆。

**不管**：**不选择用哪个模型**（「用哪个」是登记处解析链里的策略）；不落盘、不管会话。

## 二、入站契约与状态归属

`api::Llm`（造通道 / 探测 / 发现 / 修信封）+ 对外协议类型（`Chat` 的调用方是别的能力，所以契约在 `api`）。出站端口 `ChatGateway` / `ModelCatalog` / `EnvelopeRepair` **只由 `service.rs` 持有**（R12），**不进 `api`**。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`prompt`
- 谁在用我：`collab`、`conductor`、`registry`、`session`、`slate`、`tools`、`workspace`

## 四、改动本单元时必须同步

- 请求体形状是**真实会话与探针共用的唯一定义**（`detail/http_chat.rs`）；改协议 → `src/tests/llm.rs`、`src/tests/detail.rs`、`src/tests/fakes.rs`；信封回执文案来自 `prompt` 的 `ToolTexts`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |