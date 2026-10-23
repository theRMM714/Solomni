# 单元地图 · conductor

> 本文是 **conductor** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：会话在世表、命令队列、运行态与跨会话编排。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conductor/mod.rs` | 能力入口与公开面 |
| `src/capabilities/conductor/api/mod.rs` | 入站契约：`SessionOps` / `ConductorOps` / `ActionOps` / `LogOps`、`Ops` 组装面与事件台（`EventBus`） |
| `src/capabilities/conductor/api/action.rs` | 动作分发：参数校验 → 按 `callers` 授权 → 执行 → 审计（各只有一处） |
| `src/capabilities/conductor/api/handle.rs` | 核心手柄：自持线程 + 命令队列 + 事件台；`stop` 不进队列 |
| `src/capabilities/conductor/api/proxy.rs` | 队列代理：把各能力的入站契约挂到核心手柄上（呈现层因此只持 `Ops`） |
| `src/capabilities/conductor/service/mod.rs` | 会话中心：会话在世表、命令队列、运行态与派发闸（`dispatch_gate`） |
| `src/capabilities/conductor/service/action.rs` | 动作的核心实现：把规整后的建会话载荷变成一条会话 |
| `src/capabilities/conductor/service/work.rs` | 工作与会话配置：配置读写、建工作与上传、文件视图、会话视图 |
| `src/capabilities/conductor/service/turn.rs` | 回合收发与协作动作：名单分发、单 agent 的话、协作推进入口、会话生命周期 |
| `src/capabilities/conductor/service/flow.rs` | 协作流水线推进：链推进、节点派发与子会话、协作步（`take_*` / `put_*`） |
| `src/capabilities/conductor/service/rewind.rs` | 跨会话回档编排：按同一回合 id 收窗整棵子树 |
| `src/capabilities/conductor/service/env.rs` | 装配材料：成员通道、沙箱清单、工具环境、单 agent 会话构造（纯派生，不落盘） |
| `src/capabilities/conductor/service/history.rs` | 历史列出 / 打开 / 删除、事件收编与增量落盘手柄 |
| `src/capabilities/conductor/service/guest_job.rs` | 起停 guest 的两段式：核心准备参数（`GuestBoot` / `GuestStop`），调用方线程执行 |
| `src/capabilities/conductor/service/ask_user.rs` | 工具层提问端口的会话侧实现：推一条问题并阻塞等一个选项 id |
| `src/capabilities/conductor/ports.rs` | 出站端口：核心代理工具要做的外部动作（`ProxyHost`） |
| `src/capabilities/conductor/domain/action.rs` | 动作回包的纯逻辑：探测结论 → 响应 JSON |
| `src/capabilities/conductor/domain/proxy.rs` | 核心代理工具的纯逻辑：入参解析、授权校验、载荷规整与回执拼装 |
| `src/capabilities/conductor/service/proxy.rs` | 代理工具执行面与队列桥 `ProxyBridge`（把动作交给核心线程） |
| `src/capabilities/conductor/service/work_tools.rs` | 共享区六个核心自有工具（`work_pull` / `work_commit` / `work_status` / `work_versions` / `work_resolve` / `work_lost`）的成员侧执行者 |
| `src/capabilities/conductor/domain/work.rs` | 六个核心自有工具的纯逻辑：工具名、入参解析、回执拼装与三方版本 / 解决结果渲染 |
