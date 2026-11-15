# turn（会话在世状态 + 一席会话装配构件）

> 定位：会话在世表、运行态、占名表、裁决队句柄与 shell 状态槽的唯一持有者，兼一席会话的装配构件——只给 `conductor` 提供访问器与装配入口。
> 效力：规范｜核对：2026-11-15｜依据：`src/capabilities/turn/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话在世表——`SessionId → Session` 的增删查与遍历（`put` / `take` / `session` / `session_mut` / `holds` / `iter`）。
- 管：运行态——「对象被工作线程取走了、正在生成」的标记集合（`mark_running` / `is_running` / `running_iter` / `any_running`）。
- 管：占名表——准备段与提交段之间占下的名字（`mark_starting` / `is_starting` / `unmark_starting`）：同名并发创建不会都通过「名字不存在」那道检查而互相覆盖。
- 管：裁决队句柄与 shell 状态槽——`Arc<DecisionDesk>` 按会话 id 取（`desk`），shell 槽按会话 id 惰性造一份、同一会话共享（`shell_slot` / `shell_view`）。
- 不管：会话对象怎么建、回合怎么跑（`conductor`）；会话与转录怎么落盘（`session`）；裁决队的语义（`session` 的 `DecisionDesk`）与 shell 快照的形状（`session` 的 `ShellSlot` / `ShellLive`）。
- 不管：命令队列与派发闸（`conductor` 的 `service.rs`）；本单元只按 id 记账，不认识会话的内容。
- 管：一席会话的装配——按调用方交来的事实装出工具面 `member_tools` 与会话对象 `single_session`；纯规则（权限裁剪、服务操作归并、操作写进模块工具表）在 `src/capabilities/turn/domain/assemble.rs`。
- 不管：事实怎么取——权限链、沙箱、围栏、租约、计划与模块表都由 `conductor` 算好再交来；work 处理器也由调用方以闭包注入。

## 入站契约与状态归属

- 入站面：`api::TurnState`（全部访问器）与 `api::Session`（`Single` / `Collab` 两变体）；别的业务与测试只经 `api` 取用（R1）。
- 装配入站面：`api::SessionDeps`（十个能力面的 `Arc`）、`api::MemberSpec` / `api::SingleSpec`（一次装配的全部事实）、`api::member_tools` / `api::single_session`（两条装配入口），以及三条纯规则 `api::face_tools` / `api::service_ops_of` / `api::serviced_tools`。
- 装配件的状态归属：产物归调用方——`member_tools` 返回的 `MemberTools` 与 `single_session` 返回的 `(AgentSession, Vec<SessionEvent>)` 都不进五张表；本单元只在装 `MemberTools` 时向调用方的 shell 槽取一次 `Arc<ShellSlot>`。
- 状态归属：**持全部运行期内存态**——五张表都在 `TurnState` 里；不落盘，`shell_slots` 的 `Mutex` 只在取 / 写那一刻持有。
- 裁决队是 `Arc<DecisionDesk>`：生成线程与核心线程共用同一条队，所以队不随会话对象被取走而消失（工具级确认正是在「生成中」问的）。
- 运行态与占名表分开记：前者表示「对象在生成、不在表里是正常的」，后者表示「名字已占、会话还没落盘」。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`collab`（`CollabSession`）、`env`（`member` 与 `MemberFacts`）、`kernel`（`SessionId` / `ToolHandler`）、`llm`（`Channel` / `ToolMode`）、`modules`（`Module`）、`packages`（`PackagePlan`）、`permission`（`PermissionSet`）、`prompt`（`Prompt` / `Segment` / `tool_notes`）、`residents`（`ResidentOps` / `ServiceView` / `ServiceState`）、`secrets`（`SecretOps`）、`session`（`AgentSession` / `DecisionDesk` / `MemberTools` / `ModuleTools` / `ServiceOp` / `ShellLive` / `ShellSlot` / `tool_table` / `service_ops_for_modules`）、`tools`（`ToolBook` / `json_schema_lines`）、`vm`（`Vm`）、`workspace`（`Workspace`）
- 谁在用我：`conductor`（取访问器、装会话）、`src/tests/turn.rs`（状态容器与纯规则）

## 一、五张表各管什么

- 在世表：会话对象在核心手里时在里面；被工作线程取走时不在（生成中），取走与交回都经 `take` / `put`。
- 运行态：只记 id，不持对象；用于「整棵子树有生成中就拒绝」这类判定（删会话、回档、改配、切换档位前都要问它）。
- 占名表：只记名字，用于建工作的两段之间挡同名并发；提交成功或失败撤销时都要撤掉。
- 裁决队：句柄随会话 id 走，`Arc` 让等待方与改写方看同一条队；会话消失时由 `conductor` 调 `desk().forget`。
- shell 槽：工具循环写、状态栏读；同一会话拿到的是同一个 `Arc<ShellSlot>`，没跑过 shell 时快照如实为空。

## 二、会话装配构件

- 两个入口：`member_tools` 装工具面（角色脸 → 权限裁剪 → 服务操作写进模块表 → 提示词行 → 围栏与处理器），`single_session` 装单 agent 会话对象（通道、形态、上下文预算、模型、压缩工厂）。
- 只装不取：权限链、沙箱、围栏、租约、计划、基础根、提示词与模块变量都从 `SessionDeps` / `MemberSpec` / `SingleSpec` 读，函数体内无文件、无进程、无网络。
- 三条纯规则可单测：`face_tools` 按 `PermissionSet::offers` 裁剪角色脸，总表里没有的 id 照发；`service_ops_of` 只收 `ServiceState::Ready` 的操作并按服务名点操作名命名；`serviced_tools` 把操作塞进模块工具表并返回模块行与参数行。
- 处理器注入：work 等工具处理器由调用方的闭包按「本回合允许的工具 id」生成，本单元不认识它们的行为。

## 改动本单元时必须同步

- `src/capabilities/turn/api.rs` → 新增入站项要同步 [`unit-map.md`](unit-map.md) 与本页「入站契约」。
- `src/capabilities/conductor/service.rs` → `Conductor` 的 `turn` 字段是唯一持有者；改访问器签名会连带改 `service/` 下的调用方（`activation` / `flow` / `history` / `proxy` / `rewind` / `turn` / `work/config` / `work/create` / `work/model` / `work/views`）。
- `src/tests/turn.rs` → 状态容器的三条用例（运行标记按 id 收放、占名与在世表互不影响、shell 槽按会话共享）与两条纯规则用例（权限裁剪按工具决策、服务操作只出就绪项）。
- `src/capabilities/turn/service/assemble.rs` → 改 `SessionDeps` / `MemberSpec` / `SingleSpec` 字段会连带改 `conductor` 的 `session_deps` 与三个装配调用点（`src/capabilities/conductor/service/rewind.rs` / `src/capabilities/conductor/service/proxy.rs` / `src/capabilities/conductor/service/work/create.rs`）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
