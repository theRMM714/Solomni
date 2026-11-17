# work（工作用例）

> 定位：工作是什么样——建工作的两段、改配、档案与视图、模型视图、运行报告、guest 生命周期、建议与激活。
> 效力：规范｜核对：2026-11-16｜依据：`src/capabilities/work/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**建工作的两段**——`prepare_work`（核心：校验、占名、备工作区、解模块、出计划）与 `commit_work`（核心：装配会话、准入、开场事实）；中间那段慢步骤（`GuestBoot`）在调用方线程上跑，失败由 `abort_work` 撤销，占名与空目录都不留。
- 管：**改配**——`session_config` / `prepare_edit` / `commit_edit` / `set_permission_profile`：权限档、思考档、上下文新鲜度与围栏撤权。
- 管：**工作形态的落盘形状**——`WorkSpec` / `WorkOpened` / `SessionMeta` 的装配顺序、通道解析（`resolve_channels`）、工作区绑定（`resolve_workspace_binding`）与工作名校验（`domain::validate_work_name`）。
- 管：**视图**——`SessionView`（在世会话 × 历史的并集与继续判据）、`FilesView`、工作日志与提交恢复（`restore_work_commit`）。
- 管：**模型视图**——`llm_opts` / `llm_opts_for`、思考片段、树上可用思考档与按根设置。
- 管：**运行报告**——包库可用版本、缺失清单、档位视图（`RuntimeReport` / `TierChoices`）与权限视图（`PermissionView`）。
- 管：**guest 生命周期**——`GuestBoot` / `EditGuest` / `GuestDiagnose` 的起停诊断两段式（单次销毁的 `GuestStop` 归 `vm`），以及稳定根的判据（`stable_roots` / `check_stable_roots`）。
- 管：**建议与激活**——`suggest_models` 按需求拟名单、`activation_build_request` / `prepare_activation` / `commit_activation` 启用历史会话的两段。
- 不管：命令队列与生成驱动——谁在什么时候取走会话、生成完怎么放回，由 `runtime` 决定（本单元只提供用例方法）。
- 不管：会话本体、在世表与运行态（`turn`）、落盘与转录（`session`）、沙箱与工作区机制（`workspace`）、虚拟机实现与计划派生（`vm`）。
- 不管：登记处与探针本体——`&dyn Registry` / `&dyn HostProbe` 由调用方按次传入，`Work` 不持有它们（R12）。

## 入站契约与状态归属

- 入站面：`api` 的 DTO（`WorkSpec` / `WorkOpened` / `SessionConfig` / `SessionEdit` / `SessionView` / `ContinueView` / `FilesView` / `RuntimeReport` / `TierChoices` / `PermissionView` / `ThinkingView` / `AgentSuggestion` / `ConfigAgent` / `WorkMode`）与 guest 三类（`GuestBoot` / `GuestStop` / `EditGuest` / `GuestDiagnose`）；`work` 自己的 `WorkOps`、`turn` 的 `TurnOps` 与 `runtime` 的 `RuntimeOps` 签名直接用这些类型。
- 状态归属：`Work` 只持能力面（`root` / `history` / `workspace` / `vm` / `packages` / `provision` / `modules` / `llm` / `tools` / `prompt` / `systools` / `log`），**不自持会话状态**：会话在世表、运行态与裁决队都在 `turn`，经形参 `&Turn` / `&mut Turn` 读写；`registry` / `probe` 同样按次传入。
- 两段式的中间产物（`PreparedWork` / `PendingWork` / `PreparedEdit` / `PreparedActivation`）只在本能力内流转，不在 `api` 出现。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`collab`、`env`、`llm`、`modules`、`packages`、`permission`、`prompt`、`provision`、`registry`、`session`、`slate`、`tools`、`turn`、`vm`、`workspace`；`kernel` 的 `Tier` / `SessionId` / `Log` / `HostProbe` / `ExecSpec`。
- 谁在用我：`runtime`（`WorkOps` / `TurnOps` / `RuntimeOps` 的薄转发与 `Faces` 装配）、`presentation`（视图类型与 `diagnose_text`）。
- **不引用 `runtime`**：依赖只向下，`work` 在 `runtime` 之下（`layers.yaml`）。

## 一、两段式：准备与提交

- `prepare_*` 只做毫秒级的状态变换与占名（占名 `turn.mark_starting` 由调用方前后配对），慢步骤（起 guest、装运行包）由 `GuestBoot::run` 在调用方线程上跑。
- `commit_*` 落盘、装配会话、开场事实；中途失败由 `abort_work` 撤销，已建的空工作区目录一起清掉。
- 两段之间的对象是不透明结果（`PreparedWork` / `PreparedEdit` / `PreparedActivation`），调用方只能原样交给 `commit_*` 或 `abort_*`（判据在 `src/capabilities/work/service/create.rs` 的 `abort_work` 与 `src/capabilities/work/service/config.rs` 的 `commit_edit`：中间结果不提供读取口）。

## 改动本单元时必须同步

- `src/capabilities/work/api.rs` 的 DTO → `runtime` 的 `api.rs` 签名、`src/presentation/` 的视图消费方、`src/tests/routes.rs` 的路由表都要一起改。
- `src/capabilities/work/service/views.rs` → `SessionView` 的继续判据与文件视图是行为契约：改判据必须同改 `src/tests/api/session.rs`、`src/tests/routes.rs`、`src/tests/runtime/proxy.rs` 的断言。
- `src/capabilities/work/service/create.rs` → 建工作的顺序、日志 key 与失败撤销是行为契约：改一处必须同改 `src/tests/runtime.rs`、`src/tests/api/compaction.rs`、`src/tests/session/lifecycle.rs` 的断言与 e2e 的建工作旅程。
- `src/capabilities/work/service/guest_job.rs` → 稳定根判据与 guest 规格是行为契约：改一处必须同改 e2e 的 vm 档会话场景与 `SOLOMNI_VM_LIVE` 真机探针（`tests/cross-platform/gaps.yaml`）。
- `src/capabilities/work/domain.rs` → 工作名校验与自由函数（`dedup_modules` / `merge_into_one` / `mode_str` / `session_started`）是纯逻辑，行为由 e2e 的建工作与协作旅程覆盖（具体实现的用例已移除）。
- `src/capabilities/turn/turn/service/facts.rs` 的 `Turn::build_single`（装配落 `src/capabilities/conversation/session/service/assemble.rs`）→ 本单元 `commit_work` 的唯一装配入口：改签名必须同改这里、`src/capabilities/turn/turn/unit-map` 的对应行与 `Turn` 的用例。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
