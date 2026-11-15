# 单元地图 · work

> 定位：work（工作形态 + 建工作与改配构件）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/work/api.rs`

> 本文是 **work** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**工作形态的唯一实现处**：形态与名单的规整、两段式建工作与改配、会话启用与激活、guest 启停的一段活、
> 模型档与思考强度、会话 / 文件视图、运行包报告与档位都在这里；`Work` 是唯一入口，只经 `api` 对外。
> 它**不持业务状态**：会话本体（`turn`）与登记处（`registry`）按方法参数传入，转录落盘与工具面处理器由 `conductor` 注入，
> 命令队列、事件台、生成驱动与协作链推进归 `conductor`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/work.rs` | 能力入口与公开面 |
| `src/capabilities/work/api.rs` | 入站契约：`Work` 本体、`WorkSpec` / `SessionEdit` / `AgentInstance` / `ConfigAgent`、读模型（`SessionConfig` / `SessionView` / `FilesView` / `RuntimeReport` / `TierChoices` / `ThinkingView` / `ContinueView` / `PermissionView`）、两段中间形状（`PreparedActivation` / `ReadyActivation`）与 guest 句柄的对外重导出 |
| `src/capabilities/work/domain.rs` | 纯规则：形态标识（`mode_str`）、工作名校验（`validate_work_name`，含 Windows 保留设备名与非法字符）、名单保序去重（`dedup_modules`）、多个名单并成一条（`merge_into_one`）；无端口、无 IO |
| `src/capabilities/work/service.rs` | 服务层入口：`Work` 门面（注入端口、产品根、基础工具链根与钩子 `handlers`）、建造入口 `new`、只读访问器（清单扫描 / 保留名 / 虚拟根 / 历史列出与打开 / 事件落盘）、两段交接物（`PreparedWork` / `PendingWork` / `PreparedEdit`）、运行包重建判据 `needs_guest_packages_rebuild`、模块声明与名单的事实函数 |
| `src/capabilities/work/service/create.rs` | 建工作两段：名字唯一化 `unique_work_name`、子树已用名 `subtree_agent_names`、通道与工作区绑定解析、`prepare_work` / `commit_work` / `create_work` / `create_work_inner`，以及授权付款 `pays_host_grant` / `pay_workspace_grant` |
| `src/capabilities/work/service/config.rs` | 配置读取与改配两段：`session_config`、`edit_session`、`prepare_edit`（新 meta 与子树上浮）、`commit_edit`、权限档提交 `set_permission_profile`、围栏撤权 `release_fences`、配置记录 `record_config` |
| `src/capabilities/work/service/action.rs` | 建会话（不经工作形态的那条）：`create_session` / `resolve_env` / `prepare_session` / `commit_session` / `abort_session`，以及代理形态工作的会话产出 `create_proxy_with_facts`（无名单、无 guest，身份 `core_proxy`） |
| `src/capabilities/work/service/activation.rs` | 会话启用与激活两段：`activation_build_request`、`prepare_activation`（生成中直接附着 / 出 guest 启动盘与租约）、`commit_activation`（重建会话、失败按租约停 guest）、子树解冻 `resume_subtree` |
| `src/capabilities/work/service/boot.rs` | guest 的一段活与创建失败撤销：`prepare_guest_boot` / `guest_boot`（起）、`guest_stop`（停）、`abort_work`（撤占名与空工作区） |
| `src/capabilities/work/service/guest_job.rs` | 要在调用方线程跑的活：`GuestBoot`（并发准入、规格装配、运行包软链）、`GuestStop`（租约清单）、`GuestDiagnose`、`EditGuest`，虚拟根稳定根校验 `stable_roots` / `check_stable_roots`，公共祖先 `common_ancestor` |
| `src/capabilities/work/service/host.rs` | 宿主侧外部工具的取法：`qemu_path` / `virtiofsd_path` / `smbd_path` / `share_tool_path`，登记值优先、装好的兜底（`registered_or_installed`） |
| `src/capabilities/work/service/model.rs` | 一次模型调用的参数与思考档：`llm_opts` / `llm_opts_for`、思考片段 `thinking_fragment_for`、树上可用思考档 `thinking_models_in_tree` / `thinking_view_for_root`、`set_thinking`、`set_context_freshness` |
| `src/capabilities/work/service/report.rs` | 运行包报告与档位视图：`runtime_report` / `runtime_report_for`、`tier_choices`（默认档与虚拟机档的承载探针）、`unavailable_modules_now`、会话权限视图 `permission_view` |
| `src/capabilities/work/service/suggest.rs` | 拟名单入口 `suggest_models`：前置判据（登记处有无模型、核心默认是否设过）、队列分发与行解析；名单规则在 `slate` |
| `src/capabilities/work/service/views.rs` | 视图：`session_view` / `session_views`（在世会话 × 历史的并集）、`session_exists`、继续判据 `continue_action`、文件视图 `files_view`、上传 `work_upload`、日志 `work_log`、提交视图与恢复（`work_commit_view` / `work_commit_file` / `restore_work_commit`） |
