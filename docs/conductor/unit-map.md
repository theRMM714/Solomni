# 单元地图 · conductor

> 定位：conductor（协调业务）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-09｜依据：`src/capabilities/conductor/api.rs`

> 本文是 **conductor** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它与别的能力**平级**：只经各能力的 `api` 编排，**不持任何别人的端口**（R12）。
> 它拥有的是**没有任何参与方拥有**的那部分不变式：会话在世表、命令队列、运行态与跨会话编排。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/conductor.rs` | 能力入口与公开面 |
| `src/capabilities/conductor/api.rs` | 入站契约：`SessionOps` / `ConductorOps` / `ActionOps` / `LogOps`、`Ops` 组装面与事件台（`EventBus`） |
| `src/capabilities/conductor/api/action.rs` | 动作分发：参数校验 → 按 `callers` 授权 → 执行 → 审计（各只有一处） |
| `src/capabilities/conductor/api/handle.rs` | 核心手柄：自持线程 + 命令队列 + 事件台，线程边界上的短命令（补编、回档收尾、guest 启动、启用会话）；`stop` 不进队列 |
| `src/capabilities/conductor/api/generation.rs` | 生成的点火器：节点执行、继续、代理回合、协作推进、裁决处置各起一条脱离调用方的线程 |
| `src/capabilities/conductor/api/generation/single.rs` | 单 agent 与节点的生成循环：核心只占「取 / 交」两步，生成在工作线程上跑，逐轮外送并落盘 |
| `src/capabilities/conductor/api/generation/member.rs` | 成员回合：为协作泵的一个请求取该 agent 的会话、跑完把回合结果带回（子会话自己的事件台与逐轮落盘） |
| `src/capabilities/conductor/api/generation/compact.rs` | 手动压缩：让模型自己压，队列只占「取 / 交」，压不动如实说 |
| `src/capabilities/conductor/api/generation/collab.rs` | 协作泵：开始讨论 / 回答 / 继续，与主线程握手（`AskReq` / `MemberTurn`），事件边产边送 |
| `src/capabilities/conductor/api/contract_impls.rs` | 各能力入站契约的队列代理：每个方法克隆入参 → 进命令队列 → 取回结果（呈现层因此只持 `Ops`，拿不到 `Conductor`） |
| `src/capabilities/conductor/service.rs` | 会话中心：会话在世表、命令队列、运行态与派发闸（`dispatch_gate`） |
| `src/capabilities/conductor/service/action.rs` | 动作的核心实现：把规整后的建会话载荷变成一条会话 |
| `src/capabilities/conductor/service/work.rs` | 工作方法族入口：建工作的两段结果、改配准备结果、模块声明的能力表、运行包重建判据 |
| `src/capabilities/conductor/service/work/report.rs` | 运行包报告与档位视图：模块声明的能力、包库可用版本、缺失清单、平台与库、计划与权限视图 |
| `src/capabilities/conductor/service/work/config.rs` | 会话配置读写：配置视图、改配两段、权限档提交、围栏撤权、配置记录落盘 |
| `src/capabilities/conductor/service/work/model.rs` | 调用参数与思考档：流式与预算取值、思考片段、树上可用思考档、按根设置 |
| `src/capabilities/conductor/service/work/host.rs` | 宿主工具与网络开关：QEMU / virtiofsd / smbd 路径、宿主只读根；共享工具选择用 kernel 平台事实 `IS_WINDOWS` |
| `src/capabilities/conductor/service/work/views.rs` | 会话与文件视图：在世会话 × 历史的并集、继续判据、文件视图 / 上传 / 日志 / 提交恢复 |
| `src/capabilities/conductor/service/work/create.rs` | 建工作两段与 guest 启动：占名、备工作区、装通道、起 guest、失败撤销 |
| `src/capabilities/conductor/service/workspace_root.rs` | 外部工作区根的策略：按档位判网络路径 / 共享根 / 文件系统根 / UTF-8 与产品根的互包含（文本 + 落点身份），并做目录与可写探测；创建走 `resolve`，加载复检走 `recheck` |
| `src/capabilities/conductor/service/turn.rs` | 单 agent 的回合收发：工具形态刷新、生成前准备、节点派发准备、同步生成入口、继续 |
| `src/capabilities/conductor/service/suggest.rs` | 拟名单入口：按本次需求推荐 agent 名单（队列分发与行解析） |
| `src/capabilities/conductor/service/activation.rs` | 会话启用与激活两段：按转录重建为活动会话、构建请求、准备与提交 |
| `src/capabilities/conductor/service/persist.rs` | 落盘策略与增量落盘手柄：哪种会话留档、哪些事件算定稿、失败怎么报 |
| `src/capabilities/conductor/service/compact.rs` | 压缩通道工厂：全局设置指向的压缩模型解析与更新 |
| `src/capabilities/conductor/service/flow.rs` | 协作流水线推进与协作动作：链推进、节点派发与子会话、协作步（`take_*` / `put_*`）、裁决卡动作与改需求 |
| `src/capabilities/conductor/service/rewind.rs` | 跨会话回档编排：按同一回合 id 收窗整棵子树 |
| `src/capabilities/conductor/service/env.rs` | 装配材料：成员通道、沙箱清单、工具环境、单 agent 会话构造（纯派生，不落盘） |
| `src/capabilities/conductor/service/history.rs` | 历史列出 / 打开 / 删除、事件收编与增量落盘手柄；会话加载时按 meta 的档位复检外部工作区绑定 |
| `src/capabilities/conductor/service/guest_job.rs` | 起停 guest 的两段式：核心准备参数（`GuestBoot` / `GuestStop`），调用方线程执行 |
| `src/capabilities/conductor/service/ask_user.rs` | 工具层提问端口的会话侧实现：推一条问题并阻塞等一个选项 id |
| `src/capabilities/conductor/ports.rs` | 出站端口：核心代理工具要做的外部动作（`ProxyHost`） |
| `src/capabilities/conductor/domain/action.rs` | 动作回包的纯逻辑：探测结论 → 响应 JSON |
| `src/capabilities/conductor/domain/proxy.rs` | 核心代理工具的纯逻辑：入参解析、授权校验、载荷规整与回执拼装 |
| `src/capabilities/conductor/service/proxy.rs` | 代理工具执行面与队列桥 `ProxyBridge`（把动作交给核心线程） |
| `src/capabilities/conductor/service/work_tools.rs` | 共享区六个核心自有工具（`work_pull` / `work_commit` / `work_status` / `work_versions` / `work_resolve` / `work_lost`）的成员侧执行者 |
| `src/capabilities/conductor/domain/work.rs` | 六个核心自有工具的纯逻辑：工具名、入参解析、回执拼装与三方版本 / 解决结果渲染 |
