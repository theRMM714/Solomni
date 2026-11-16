# 单元地图 · runtime

> 定位：runtime（顶层编排）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/runtime/api.rs`

> 本文是 **runtime** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**（表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：命令队列、门面与动作分发、跨会话编排；会话在世表、运行态、回合推进、事件台与落盘归 `turn`。
> 「工作是什么样」的用例（建工作两段、改配、视图、运行报告、guest 生命周期、建议与激活）在 `work`（见 [../work/unit-map.md](../work/unit-map.md)），本单元只留应用编排与同名薄转发。
> 「历史生命周期」的用例（回档三模式、历史列表与只读打开、删除的报账与收尾）在 `lifecycle`（见 [../lifecycle/unit-map.md](../lifecycle/unit-map.md)），本单元只给时机。
> 「核心代理工具」的工具面（六工具的载荷、授权判据、幂等账与 `ProxyHandler`）与宿主契约 `ProxyHost` 在 `proxy`（见 [../proxy/unit-map.md](../proxy/unit-map.md)），本单元只留队列桥 `ProxyBridge` 与它的装配。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/runtime.rs` | 能力入口与公开面 |
| `src/capabilities/runtime/api.rs` | 入站契约：`RuntimeOps`（启用 / 改需求 / 回档 / 停止）、`ActionOps`（动作目录与分发）、`LogOps` 与动作词汇（`Caller` / `ActionCall` / `ActionView` / `Acted`）；呈现层共用读模型（`SessionEvent` / `DecisionCard` / `Tier` / `Advance` / `EventBus` / `Output`）经这里重导出 |
| `src/capabilities/runtime/service.rs` | 子模块声明与共用 import（各能力 `api` 的类型、压缩工厂、动作词汇）；不含任何用例身体 |
| `src/capabilities/runtime/service/core.rs` | 核心本体 `Core`：注入面字段、17 参构造 `Core::new`、访问器（`registry` / `registry_mut` / `log_handle` / `tools_handle` / …）、`registry_write`、`sync_compact_choice`，以及 31 个同名薄转发 |
| `src/capabilities/runtime/service/handle.rs` | 核心手柄 `Handle`：`spawn` 把 `Core` 搬进 `kernel` 的 `Actor`（线程名 `solomni-core`）、`call` / `publish` / `record_notice` / `events` / `allow_tool_cards`，以及 `TurnHost` 的宿主侧实现 |
| `src/capabilities/runtime/service/contracts.rs` | 各入站契约在 `Handle` 上的实现：`WorkOps` / `TurnOps` / `RuntimeOps` / `RegistryOps` / `HistoryOps` / `ModulesOps` / `LogOps`，每方法一行 `self.call(...)` 或一行转发 |
| `src/capabilities/runtime/service/activation.rs` | 长步骤两段式的调用方线程侧：补编 `ensure_built`、guest 启动租约 `run_guest_boot` / `ensure_guest`、启用会话 `activate_session` |
| `src/capabilities/runtime/service/rewind.rs` | 回档编排：`rewind_with_reset` 先 `settle_tree` 停稳子树再回档，`stop_tree` / `void_gates` / `publish_rewound` |
| `src/capabilities/runtime/service/bridge.rs` | 代理工具的队列桥 `ProxyBridge`（实现 `proxy` 的 `ProxyHost`）、`Core` 的 `proxy_*` 视图族与 `inject_proxy_handler` |
| `src/capabilities/runtime/service/action.rs` | 动作分发主干：`base64_decode`、`authorize_action` / `audit_action` / `execute_action` 的分派与 `ActionOps` 实现，以及建会话载荷的核心两步（`prepare_session` / `commit_session`） |
| `src/capabilities/runtime/service/action/session.rs` | 建会话与控会话动作：`create_session_call` / `control_session_call` / `build_new_session` |
| `src/capabilities/runtime/service/action/module.rs` | 模块工具动作：可用性判定、授权、清单解析与执行（`run_module_action`） |
| `src/capabilities/runtime/service/action/registry.rs` | 登记处动作与设置：`execute_registry` / `apply_settings` / `probe_json`，以及探测回包的纯逻辑 `probe_view` 与其单测 |
| `src/capabilities/runtime/service/env.rs` | guest 诊断准备与两席沙箱 / 权限薄入口（事实在 `turn::service::facts`） |
| `src/capabilities/runtime/service/compact.rs` | 压缩通道工厂 `CompactRuntime`：全局设置指向的压缩模型解析与更新 |
