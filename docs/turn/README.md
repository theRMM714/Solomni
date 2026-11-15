# turn（会话本体 + 一席会话装配构件）

> 定位：会话本体——在世表、运行态、占名表、裁决队句柄与 shell 状态槽的唯一持有者，也是「一席会话怎么装出来、怎么从落盘重建、树序与身份怎么算」的唯一实现处；只给 `conductor` 提供入站面。
> 效力：规范｜核对：2026-11-15｜依据：`src/capabilities/turn/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话在世表——`SessionId → Session` 的增删查与遍历（`Turn::put` / `take_session` / `session` / `session_mut` / `holds` / `iter`）。
- 管：运行态——「对象被工作线程取走了、正在生成」的标记集合（`mark_running` / `is_running` / `running_iter` / `any_running`）。
- 管：占名表——准备段与提交段之间占下的名字（`mark_starting` / `is_starting` / `unmark_starting`）：同名并发创建不会都通过「名字不存在」那道检查而互相覆盖。
- 管：裁决队句柄与 shell 状态槽——`Arc<DecisionDesk>` 按会话 id 取（`desk_of` / `forget_desk`），shell 槽按会话 id 惰性造一份、同一会话共享（`shell_slot` / `shell_view`）。
- 管：取放协议——`take_single` / `take_collab` / `put_single` / `put_single_recorded` / `put_collab` / `abort_running`：派发闸、不是单 agent 模式、无此会话、生成中这几类拒绝的文案都在这里逐字定下；取走即标生成中，交回即撤标记。
- 管：裁决队作废——`void_gates_of` / `void_gates_in_subtree`：停止与关闭两种理由的文案在这里，卡片本身由 `session` 的队提供。
- 管：会话装配——`ensure_session`（不在表里就按转录重建再装回）、`spawn_agent_session` / `spawn_sub_session`（从父会话的名单或链节点派生）、`single_session` / `member_tools`（一席工具面与会话对象的装配入口）。
- 管：从落盘重建——`rebuild_session` 按 meta 与转录把三种形态（single / collab / proxy）整份重建，`replay_dialogue` / `replay_reads` 是发送视图与读事实的回放规则。
- 管：树序与身份——`subtree_of` / `work_root_of` / `work_root`（父子成环的拒绝文案在这里）/ `session_kind_of` / `role_of` / `tree_meta` / `tree_meta_with` / `workspace_binding_of` / `dispatch_gate` / `dispatch_target`。
- 管：装配要的事实——沙箱集合、权限链与权限面、共享区授权、guest 作用域与租约、配置归属、上下文预算（`sandboxes` / `permissions_for` / `workspace_grant` / `guest_scope` / `guest_lease_for` / `config_owner_of` / `context_budget` / `refresh_context_budget`）。
- 管：执行计划与外部工作区根——`host_plan_for` / `plan_for_spec` / `plan_for_meta_with` / `unavailable_modules` / `guest_os_of` / `path_codec_of` / `network_allowed` / `host_read_roots`。
- 不管：命令队列、派发闸的调用时机、跨会话回档编排、协作链推进与生成驱动（`conductor`）；会话与转录怎么落盘（`session`）；裁决队的语义（`session` 的 `DecisionDesk`）与 shell 快照的形状（`session` 的 `ShellSlot` / `ShellLive`）。
- 不管：登记处与探针——`registry: &dyn Registry` / `probe: &dyn HostProbe` 按需传参、不存字段（`conductor` 要 `registry_mut`，共享会逼出 `RefCell`）；work 处理器也由调用方以闭包注入，本单元不认识它们的行为。

## 入站契约与状态归属

- 入站面：`api::Turn`（会话本体：五张表的访问器 + 取放协议 + 装配与重建 + 树序与事实）、`api::TurnState` / `api::Session`（状态容器与两种会话变体）；别的业务与测试只经 `api` 取用（R1）。
- 装配入站面：`api::MemberSpec` / `api::SingleSpec`（一次装配的全部事实）、`api::member_tools` / `api::single_session`（两条装配入口）、`api::ToolHandlers` / `api::HandlerSource`（处理器注入的返回集合与闭包类型），以及三条纯规则 `api::face_tools` / `api::service_ops_of` / `api::serviced_tools`。
- 树与事实入站面：`api::flat_modules` / `api::session_started`（纯规则）、`api::PROXY_PRIVATE_DIR`（代理会话的私有目录名）、`api::replay_dialogue`（转录回放，测试与重建同一份实现）。
- 装配件的状态归属：产物归调用方——`member_tools` 返回的 `MemberTools` 与会话对象都不进五张表；本单元只在装 `MemberTools` 时向调用方的 shell 槽取一次 `Arc<ShellSlot>`。
- 状态归属：**持全部运行期内存态**——五张表都在 `TurnState` 里，由 `Turn` 独占持有；不落盘，`shell_slots` 的 `Mutex` 只在取 / 写那一刻持有。
- 不落盘的边界：`put_single` / `put_collab` / `notify_proxy_of_child` 只把「该写什么」作为事件返回，落盘由调用方 `conductor` 调 `record_events` 完成——本单元不持落盘手柄、不新增端口。
- 裁决队是 `Arc<DecisionDesk>`：生成线程与核心线程共用同一条队，所以队不随会话对象被取走而消失（工具级确认正是在「生成中」问的）。
- 运行态与占名表分开记：前者表示「对象在生成、不在表里是正常的」，后者表示「名字已占、会话还没落盘」。
- 依赖方向单向：只允许 `conductor → turn`；本单元源码不得出现 `capabilities::conductor`（依赖自检见本页「改动本单元时必须同步」）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`collab`（`CollabSession`）、`env`（`member` / `MemberFacts` / `SandboxesFacts` / `policy` / `guest_lease` / `permissions` / `workspace_grant` / `sandboxes`）、`kernel`（`SessionId` / `ToolHandler` / `PathCodec` / `Tier` / `view_codec` / `native`）、`lifecycle`（`HistoryView` / `list` / `subtree_of` / `open`）、`llm`（`Channel` / `Llm` / `Msg` / `ToolMode`）、`modules`（`Module` / `Modules` / `Roster`）、`packages`（`PackagePlan` / `PinPolicy` / `plan_guest` / `plan_host` / `library_for_guest`）、`permission`（`PermissionSet`）、`prompt`（`Prompt` / `Segment` / `tool_notes`）、`provision`（虚拟根清单只经 `conductor` 传入的事实）、`registry`（`Registry`：通道、工具形态、快照、应用设置）、`residents`（`ResidentOps` / `ServiceView` / `ServiceState`）、`secrets`（`SecretOps`）、`session`（`AgentSession` / `History` / `ReadFact` / `SessionEvent` / `SessionMeta` / `DecisionDoor` / `MemberTools` / `ModuleTools` / `ServiceOp` / `ShellLive` / `ShellSlot` / `tool_table` / `service_ops_for_modules`）、`tools`（`ToolBook` / `ToolExec` / `Tools` / `json_schema_lines`）、`vm`（`Vm` / `resolve_guest` / `classify_base` / `PathBridge` / `guest_platform` / `missing_entries_for` / `plan`）、`workspace`（`Workspace` / `Sandbox`）
- 谁在用我：`conductor`（经 `Turn` 读写会话、装配与重建、取树序与事实）、`src/tests/turn.rs`（状态容器与纯规则）、`src/tests/*`（重建、树序、装配的用例）

## 一、五张表各管什么

- 在世表：会话对象在核心手里时在里面；被工作线程取走时不在（生成中），取走与交回都经 `take_session` / `put`。
- 运行态：只记 id，不持对象；用于「整棵子树有生成中就拒绝」这类判定（删会话、回档、改配、切换档位前都要问它）。
- 占名表：只记名字，用于建工作的两段之间挡同名并发；提交成功或失败撤销时都要撤掉。
- 裁决队：句柄随会话 id 走，`Arc` 让等待方与改写方看同一条队；会话消失时由 `conductor` 调 `forget_desk`。
- shell 槽：工具循环写、状态栏读；同一会话拿到的是同一个 `Arc<ShellSlot>`，没跑过 shell 时快照如实为空。

## 二、会话装配、重建与取放

- 两个装配入口：`member_tools` 装工具面（角色脸 → 权限裁剪 → 服务操作写进模块表 → 提示词行 → 围栏与处理器），`single_session` 装单 agent 会话对象（通道、形态、上下文预算、模型、压缩工厂）。
- 只装不取：权限链、沙箱、围栏、租约、计划、基础根、提示词与模块变量都从入参读；函数体自身不碰文件、不起进程、不走网络（路径与平台事实一律来自 `probe` / `Workspace` / `Vm` 这些端口，装配入口见 `src/capabilities/turn/service/assemble.rs`）。
- 重建一整条路：`ensure_session` = 在表里就返回、生成中就拒、否则按 `history` 的 meta 与转录走 `rebuild_session` 再装回表；三种形态各走各的装配，未知形态的文案逐字保留。
- 取放协议：取走先过派发闸与「不是单 agent 模式」判定，交回时按父会话与节点把提示写进事件交回给调用方，子会话结束还会给代理父会话留一条提醒（同样以事件返回）。
- 三条纯规则可单测：`face_tools` 按 `PermissionSet::offers` 裁剪角色脸，总表里没有的 id 照发；`service_ops_of` 只收 `ServiceState::Ready` 的操作并按服务名点操作名命名；`serviced_tools` 把操作塞进模块工具表并返回模块行与参数行。
- 处理器注入：work 等工具处理器由调用方的闭包按「本回合允许的工具 id」生成，本单元不认识它们的行为。

## 改动本单元时必须同步

- `src/capabilities/turn/api.rs` → 新增入站项要同步 [`unit-map.md`](unit-map.md) 与本页「入站契约」。
- `src/capabilities/conductor/service.rs` → `Conductor` 的 `turn` 字段是唯一持有者（类型是 `api::Turn`，不再自带会话段方法）；改 `Turn` 的方法签名会连带改 `conductor/service/` 下的调用方（`activation` / `flow` / `history` / `proxy` / `rewind` / `turn` / `work/config` / `work/create` / `work/model` / `work/views`）与 `api/generation/*`。
- `src/capabilities/turn/service/assemble.rs` → 改 `MemberSpec` / `SingleSpec` 字段会连带改 `conductor` 的三个装配调用点（`rebuild_session` / `build_proxy` / `work/create`）。
- `src/capabilities/workspace/api.rs` → 外部工作区根的判据（`root_policy` / `resolve_root` / `recheck_root` / `product_root`）在 `workspace`；改文案会连带改本单元的加载复检与 `conductor` 的建工作两条路径。
- 依赖自检：`grep -r "capabilities::conductor" src/capabilities/turn` 必须为 0 命中（本单元不得回引 `conductor`）。
- `src/tests/turn.rs` → 状态容器的三条用例（运行标记按 id 收放、占名与在世表互不影响、shell 槽按会话共享）与两条纯规则用例（权限裁剪按工具决策、服务操作只出就绪项）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
