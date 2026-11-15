# work（工作形态 + 建工作与改配构件）

> 定位：工作形态的唯一实现处——把「要一份工作」翻译成会话配置（形态、名单、模型档、权限、虚拟根），并把建工作、改配、视图、运行报告与 guest 启停一并收在这里；只给 `conductor` 提供入站面。
> 效力：规范｜核对：2026-11-15｜依据：`src/capabilities/work/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：工作形态——single / collab / proxy 三条入口的形态决定（`WorkSpec` / `WorkMode`）与名单规整；proxy 没有名单，会话实现上是一条单会话、身份换成 `core_proxy`，用户选它就是授予全权。
- 管：建工作两段——`prepare_work`（校验、占名、备工作区、解析通道与工作区绑定、出 guest 计划）与 `commit_work`（落盘、装配会话、开场事实）；失败撤销是 `abort_work`。
- 管：guest 的一段活——`prepare_guest_boot` / `guest_boot`（起）、`guest_stop`（停）、`GuestDiagnose`（诊断）；参数准备在核心，`run()` 在调用方线程。
- 管：会话启用与激活两段——`activation_build_request` / `prepare_activation` / `commit_activation`，以及子树解冻 `resume_subtree`。
- 管：建会话（不经工作形态的那条）——`create_session` / `resolve_env` / `prepare_session` / `commit_session` / `abort_session`，含代理形态工作的会话产出 `create_proxy_with_facts`。
- 管：配置读取与改配——`session_config` / `edit_session` / `prepare_edit` / `commit_edit` / `set_permission_profile` / `release_fences` / `record_config`。
- 管：模型档与思考强度——`llm_opts` / `llm_opts_for`、`thinking_fragment_for` / `thinking_models_in_tree` / `thinking_view_for_root` / `set_thinking` / `set_context_freshness`。
- 管：视图与报告——`session_view` / `session_views` / `session_exists` / `continue_action`、文件视图与上传 / 日志 / 提交与回滚恢复、`runtime_report` / `tier_choices` / `permission_view`。
- 管：拟名单入口 `suggest_models`（前置判据、队列分发、行解析；名单的业务规则在 `slate`）。
- 管：纯规则与宿主工具取法——`mode_str` / `validate_work_name` / `dedup_modules` / `merge_into_one`（`src/capabilities/work/domain.rs`）、`qemu_path` / `virtiofsd_path` / `smbd_path` / `share_tool_path`（`src/capabilities/work/service/host.rs`）。
- 不管：会话本体与运行态——在世表、运行态、占名表、裁决队句柄与 shell 槽在 `turn` 的 `Turn`（`src/capabilities/turn/service/state.rs`）；本单元只经方法参数 `turn` 读写。
- 不管：命令队列、事件台、生成驱动、协作链推进与跨会话回档编排（`conductor`）；转录落盘归 `session`（`session::api::record_events`），工具面处理器也不实现——成员侧工作工具处理器由 `turn` 装配时按放行名单向 `workspace` 现取。
- 不管：guest 进程（`vm`）、工作区文件与提交（`workspace`）、包库与模块清单（`packages` / `modules`）、依赖补齐（`provision`）、登记处与探针（`registry` / `HostProbe`）——一律走各自 `api`，本单元不持它们的端口（R12；`Work` 的字段表见 `src/capabilities/work/service.rs`）。

## 入站契约与状态归属

- 入站面：`api::Work`（本能力唯一入口）、`api::WorkSpec` / `api::SessionEdit`（入站决定）、`api::AgentInstance` / `api::ConfigAgent`（名单项）、读模型 `api::SessionConfig` / `api::SessionView` / `api::FilesView` / `api::RuntimeReport` / `api::TierChoices` / `api::ThinkingView` / `api::ContinueView` / `api::PermissionView`。
- 跨到调用方线程的入站项：`api::GuestBoot` / `api::GuestStop` / `api::GuestDiagnose`（`run()` 在调用方线程执行）、两段中间形状 `api::PreparedActivation` / `api::ReadyActivation`。
- 状态归属：**不持业务状态**。`Work` 只有注入的端口、产品根与基础工具链根；会话本体经 `turn`、登记处经 `registry` 按方法参数传入——同一个核心既有只读调用也有 `registry_mut` 写路径，把登记处存进字段会逼出内部可变性。
- 只在核心线程存在的中间形状：`PreparedWork` / `PendingWork` / `PreparedEdit`（`pub(crate)`，`api` 不导出）——它们是「准备完、还没提交」的交接物，提交或撤销后即丢弃。
- 落盘写点不在本单元：`record_events` 转调 `session` 的 `record_events`（失败如实变成一条 `SessionEvent::Notice`）；历史列表与打开转调 `lifecycle` 的 `list` / `open`。
- 依赖方向单向：只允许 `conductor → work`；本单元源码不得出现 `capabilities::conductor`（依赖自检见本页「改动本单元时必须同步」）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`collab`（`CollabSession`）、`kernel`（`api` 的 `SessionId` / `Tier` / `FenceSpec` / `ExecSpec` / `now_ts` 与 `ports` 的 `HostProbe` / `Log`）、`lifecycle`（`list` / `open`）、`llm`（`Channel` / `Llm`）、`modules`（`Module` / `Modules` / `Roster`）、`packages`（`Packages` / 计划与选型诊断）、`permission`（`ProfileId` / `Reach` / `Decision` / `PermissionLayer` / `ProfileSpec` / `ScopeMode`）、`prompt`（`Prompt`）、`provision`（`Provision` / `GuestBase`）、`proxy`（`NewSession` / `Created`：建会话的入站载荷）、`registry`（`Registry` / `AgentView`）、`session`（`AgentMeta` / `History` / `RunState` / `SessionEvent` / `SessionMeta` / `WorkspaceGrant`）、`slate`（`propose`）、`tools`（`Tools` / `ToolExec`）、`turn`（`Turn` / `Session`）、`vm`（`Vm` / `tier_readiness` / `VmRequirement` / 虚拟根解析）、`workspace`（`Workspace` / `Sandbox` / `Sandboxes` / `WorkUsage`）
- 谁在用我：`conductor`（`WorkOps` 转发与句柄层的两段调用）、`cli` / `web`（`WorkSpec` 与各类视图类型，数据仍经 `Ops` 取）、`src/tests/*`（建工作、改配、激活、guest、视图的用例）

## 一、两段式：核心只做毫秒级

- 四条用例都是「准备 → 调用方线程跑慢活 → 提交」：建工作（`prepare_work` / `commit_work`）、建会话（`prepare_session` / `commit_session`）、启用历史会话（`prepare_activation` / `commit_activation`）、改配（`prepare_edit` / `commit_edit`）。
- 准备段做完全部可能失败的事：校验、占名、解析通道、备工作区、出计划；提交段只做落盘与装配，不再可能因「没模型」这类原因失败。
- 失败撤销是准备段的对偶：`abort_work` 撤占名与空工作区目录，`abort_session` 撤会话准备的半成品。

## 二、三条入口共用一份规整

- 名字唯一化 `unique_work_name`、子树已用名 `subtree_agent_names`、名单保序去重 `dedup_modules`、名字合法性 `validate_work_name` 都在这里，`create_session` 与 `prepare_work` 共用同一套规则。
- 代理形态走 `create_proxy_with_facts`：没有名单、没有 guest，会话直接以 `core_proxy` 身份装出（装配在 `turn`，本单元只给事实）。
- 通道解析在准备段一次性做完并存进 `PendingWork`：提交段拿到的已是可用的 `Channel`。

## 三、guest 的一段活为什么在调用方线程

- 起一台虚拟机会话要等外部世界；核心照常应答别的会话、状态查询与配置读取，所以 `GuestBoot` / `GuestStop` / `GuestDiagnose` 只由核心准备参数，`run()` 由调用方在调用方线程执行（`src/capabilities/work/service/guest_job.rs`）。
- 删会话同样两段：核心收集租约与待删目录，调用方线程先停 guest 再删目录（Windows 上 guest 可能还占着目录里的文件）。
- 虚拟根的稳定根校验 `check_stable_roots` 与 `GuestStop` 同行：起停都在同一处，路径漂移的判定不散落。

## 改动本单元时必须同步

- `src/capabilities/work/api.rs` → 新增入站项要同步 [`unit-map.md`](unit-map.md) 与本页「入站契约」；改 `SessionConfig` / `SessionView` / `FilesView` 字段会连带改 `cli` / `web` 的渲染与 `src/tests/routes.rs` 的契约矩阵。
- `src/capabilities/work/service.rs` → 改 `Work::new` 的端口会连带改 `src/capabilities/conductor/service.rs` 的装配；转录落盘在 `session::api` 一侧；改 `PreparedWork` / `PendingWork` / `PreparedEdit` 会连带改 `create.rs` / `config.rs` 与 `src/capabilities/work/service/action.rs` 的同步入口。
- `src/capabilities/conductor/api/contract_impls.rs` → `WorkOps` 的每个方法都转发到这里的同名方法，方法改名或改参数必须两处同改（呈现层只认 `WorkOps`，不认这里）。
- `src/capabilities/conductor/service.rs` → `Conductor` 的 `work` 字段是唯一持有者，调用形态是 `self.work.xxx(&mut self.turn, &*self.registry, …)`；`turn` 与 `registry` 按参数传入，不做字段。
- `src/capabilities/turn/api.rs` → 本单元用 `Turn` / `Session`；装配入口的签名变化会连带改 `src/capabilities/work/service/activation.rs` 与 `src/capabilities/work/service/action.rs`。
- `src/capabilities/work/domain.rs` → 工作名规则与 `systools/names.yaml` 的保留名同源；改文案会连带改 `src/tests/*` 里断言错误文案的用例。
- 依赖自检：`grep -r "capabilities::conductor" src/capabilities/work` 必须为 0 命中（本单元不得回引 `conductor`）。
- 测试：`src/tests/session/edit.rs` / `src/tests/session/vm.rs` / `src/tests/vm/*.rs` / `src/tests/workspace/tier.rs` 覆盖建工作、改配、激活与 guest 两段；纯规则用例在 `src/capabilities/work/service/create.rs` 与 `src/tests/turn.rs` 就近登记。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
