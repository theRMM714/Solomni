# 单元地图 · work

> 定位：work（工作用例）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/work/api.rs`

> 本文是 **work** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**工作用例的唯一落点**：建工作的两段、改配、视图、模型视图、运行报告、guest 生命周期、建议与激活都只在这里实现。
> 它**不持会话状态、不持端口**：会话在世表 / 运行态 / 裁决队归 `turn`，登记处与探针按次由 `runtime` 以形参传入。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/work.rs` | 能力入口与公开面 |
| `src/capabilities/work/api.rs` | 入站 DTO 与 guest 三类：`WorkSpec` / `WorkOpened` / `WorkMode` / 视图与报告、`GuestBoot` / `EditGuest` / `GuestDiagnose`、两段式中间结果 `PreparedActivation` / `ReadyActivation` |
| `src/capabilities/work/domain.rs` | 建工作的纯逻辑：工作名校验、模块去重、单会话合并、工作形态文案、开场事实判定 |
| `src/capabilities/work/service.rs` | 工作方法族的入口：`Work` 的能力面与两段结果（`PreparedWork` / `PendingWork` / `PreparedEdit`）、模块声明的能力表、运行包重建判据 |
| `src/capabilities/work/service/create.rs` | 建工作两段与 guest 启动：占名、备工作区、装通道、起 guest、失败撤销；代理会话的创建也在这一处 |
| `src/capabilities/work/service/config.rs` | 会话配置读写：配置视图、改配两段、权限档提交、围栏撤权、配置记录落盘 |
| `src/capabilities/work/service/views.rs` | 会话与文件视图：在世会话 × 历史的并集、继续判据、文件视图 / 上传 / 日志 / 提交恢复 |
| `src/capabilities/work/service/model.rs` | 调用参数与思考档：流式与预算取值、思考片段、树上可用思考档、按根设置 |
| `src/capabilities/work/service/report.rs` | 运行包报告与档位视图：模块声明的能力、包库可用版本、缺失清单、权限视图；平台 / 库判定与计划派生在 `vm` 的 `plan`，本文件只留 `plan_for_meta_with` 取会话与名册后转调 |
| `src/capabilities/work/service/host.rs` | 宿主工具路径：QEMU / virtiofsd / smbd 与共享工具选择（用 kernel 平台事实 `IS_WINDOWS`） |
| `src/capabilities/work/service/guest_job.rs` | 起停 guest 的两段式：核心准备参数（`GuestBoot` / `GuestDiagnose`），调用方线程执行；稳定根判据（`stable_roots` / `check_stable_roots`）也在这里 |
| `src/capabilities/work/service/suggest.rs` | 拟名单入口：按本次需求推荐 agent 名单（队列分发与行解析在 `runtime`） |
| `src/capabilities/work/service/activation.rs` | 会话启用与激活两段：构建请求、准备与提交；按转录重建与准入转调 `turn`（`ensure_session` / `resume_subtree`） |
