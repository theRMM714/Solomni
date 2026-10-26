# 单元地图 · llm

> 本文是 **llm** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/llm/api.rs` | 入站能力面：通道与协议词汇（`Chat` / `Msg` / `Completion` / `Channel` / `ToolMode` / `LlmOpts` …）与 `Llm` 用例面，并重导出信封解析与不合法信封的回执装配 |
| `src/capabilities/llm/service.rs` | 本能力的用例与端口持有者：持 `ChatGateway` / `ModelCatalog` / `EnvelopeRepair` 三个出站端口，按 `Llm` 用例答话 |
| `src/capabilities/llm/ports.rs` | 出站端口：通道工厂 `ChatGateway`、模型目录 `ModelCatalog`、信封修复 `EnvelopeRepair`（只由 `service.rs` 持有） |
| `src/capabilities/llm/detail/http_chat.rs` | 真实通道：OpenAI 兼容 `/chat/completions` 的 `HttpChat` 会话与 `HttpGateway` 工厂（端点回落、原生工具调用、非流式调用）；流式读在 `http_stream`；只接收已解析通道，不回落假模型 |
| `src/capabilities/llm/detail/http_stream.rs` | 可取消的流式 HTTP 适配器：reqwest + current-thread tokio，SSE 解析、首响应/首块/流空闲分层超时与取消关闭 |
| `src/capabilities/llm/detail/http_probe.rs` | 原生工具调用与回放形状的实测探针：发两条最小请求判定支持，并逐项报「收了没有 / 看懂了没有」 |
| `src/capabilities/llm/detail/repair.rs` | 默认的信封修复器：只做无歧义修补（转义裸控制字符、补扫描器算出的收尾括号），断在字符串中间一律不修 |
| `src/capabilities/llm/detail/http_agent.rs` | llm 侧出站 HTTP 包装：为探测/模型清单构建 ureq 代理（TLS 走 kernel/outbound）、统一判状态、脱敏与出站错误性质分类 |
| `src/capabilities/llm/detail/endpoint.rs` | 端点候选、重试判定与尝试归类（纯逻辑，无 IO），外加进程内共享的端点记忆 |
| `src/capabilities/llm/detail/model_catalog.rs` | `ModelCatalog` 的适配器：OpenAI 兼容 `GET {base_url}/models`，按候选补全与回落，错误先脱敏 |
| `src/capabilities/llm/detail/fake_chat.rs` | 测试专用通道：脚本假模型（无网络、无密钥），兼作 `Chat` 与 `ChatGateway`，只给单元测试与测试替身回放 |
| `src/capabilities/llm/domain/envelope.rs` | 发言信封的解析（纯逻辑）：认出最外层结构（发言 / 表态 / 工具调用），并给出不合法信封的判定类别与收尾状态 |
| `src/capabilities/llm/domain/malformed.rs` | 不合法信封的回执装配：按判定类别给出对应修法；单数形态与 `calls` 数组形态都示范（文案归本模块，不经提示词册） |
