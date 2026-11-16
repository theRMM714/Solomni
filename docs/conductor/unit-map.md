# 单元地图 · conductor

> 定位：conductor（协调业务）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/conductor/api.rs`

> 本文是 **conductor** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：命令队列、门面与动作分发、跨会话编排；会话在世表、运行态、回合推进、事件台与落盘归 `turn`。
> 「工作是什么样」的用例（建工作两段、改配、视图、运行报告、guest 生命周期、建议与激活）已下沉到 `work`（第 9 层），本单元只留应用编排与同名薄转发（见 [../work/unit-map.md](../work/unit-map.md)）。
> 「历史生命周期」的用例（回档三模式、历史列表与只读打开、删除的报账与收尾）已下沉到 `lifecycle`（第 9 层，见 [../lifecycle/unit-map.md](../lifecycle/unit-map.md)），本单元只给时机。
> 「核心代理工具」的工具面（六工具的载荷、授权判据、幂等账与 `ProxyHandler`）与宿主契约 `ProxyHost` 已下沉到 `proxy`（第 9 层，见 [../proxy/unit-map.md](../proxy/unit-map.md)），本单元只留队列桥 `ProxyBridge` 与它的装配。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conductor.rs` | 能力入口与公开面 |
| `src/capabilities/conductor/api.rs` | 入站契约：`SessionOps` / `ConductorOps` / `ActionOps` / `LogOps` 与 `Ops` 组装面；回合推进的值类型与事件台经 `turn::api` 重导出 |
| `src/capabilities/conductor/api/action.rs` | 动作分发：参数校验 → 按 `callers` 授权 → 执行 → 审计（各只有一处） |
| `src/capabilities/conductor/api/handle.rs` | 核心手柄：装配命令队列（`kernel` 的 `Actor`，线程与 panic 隔离在它里面）与事件台（`turn` 的 `EventBus`），线程边界上的短命令（补编、回档收尾、guest 启动、启用会话）；`stop` 不进队列；`TurnHost` 的宿主侧实现在文件末尾 |
| `src/capabilities/conductor/api/contract_impls.rs` | 各能力入站契约的队列代理：每个方法克隆入参 → 进命令队列 → 取回结果（呈现层因此只持 `Ops`，拿不到 `Conductor`） |
| `src/capabilities/conductor/service.rs` | 只管命令队列侧：能力面字段、构造与薄转发、`registry_write`、`sync_compact_choice` 与回档重建的落点；会话本体（`Turn`）、在世表 / 运行态 / 裁决队 / shell 槽、回合推进、事件台与落盘都在 `turn` 里，工作用例的实现在 `work` 里 |
| `src/capabilities/conductor/service/action.rs` | 动作的核心实现：把规整后的建会话载荷变成一条会话 |
| `src/capabilities/conductor/service/compact.rs` | 压缩通道工厂：全局设置指向的压缩模型解析与更新 |
| `src/capabilities/conductor/service/env.rs` | 薄转发：guest 诊断准备与沙箱 / 权限 / 预算三个薄入口（事实在 `turn::service::facts`）；单会话装配（`Turn::build_single`）已归 `turn` |
| `src/capabilities/conductor/domain/action.rs` | 动作回包的纯逻辑：探测结论 → 响应 JSON |
| `src/capabilities/conductor/service/proxy.rs` | 队列桥 `ProxyBridge`：实现 `proxy` 的 `ProxyHost`，把宿主动作交回核心线程（工具面与幂等账在 `proxy`）；代理会话的装配在 `turn::service::rebuild`（`build_proxy`） |
