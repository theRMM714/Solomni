# 单元地图 · prompt

> 本文是 **prompt** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/prompt/api.rs` | 入站能力面：`Prompt` 能力面（按名字取段 / 渲染、机制与环境块、共享的 `ToolTexts` / `RefsPrompts`）与相应 DTO |
| `src/capabilities/prompt/service.rs` | `Prompt` 能力面的实现与装配入口：把 `api::Prompt` 挂到纯数据的 `Prompts` 上（全进程只有这一处持有者） |
| `src/capabilities/prompt/ports.rs` | 出站端口：提示词册的加载 `PromptSource` |
| `src/capabilities/prompt/domain/prompt.rs` | 提示词册的纯数据与渲染层：`{{key}}` 占位替换、段选择、机制 / 工作环境匹配与册子合并（缺键缺变量如实报错） |
| `src/capabilities/prompt/domain/refs.rs` | 用户输入里 `@work:` / `@sandbox:` 引用到真实路径的改写（纯逻辑）：认前缀、按会话环境写法渲染，越界判定留给 workspace |
| `src/capabilities/prompt/detail/yaml_prompts.rs` | `PromptSource` 的磁盘实现：读 `prompts/` 下的 yaml 并合并成册子（缺目录 / 缺文件如实报装配错误） |
