# runtime（顶层编排）

> 定位：最顶层只做编排——命令队列与门面、长步骤的两段式、动作分发、代理工具的宿主桥；用例的身体一律留在内环单元。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/business/runtime/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话中心——命令队列与派发闸；会话在世表、运行态与创建占位在 `turn_state` 里，只经它的 `api` 读写。
- 管：**回合推进的宿主侧**——生成 / 协作泵 / 压缩的线程由队列手柄点火（线程名 `solomni-*` 不变）；推进规则、事件台与逐轮落盘在 `turn`（见 [../turn/README.md](../turn/README.md)）。
- 管：**代理工具的宿主与时序**（`src/units/business/runtime/service/bridge.rs`）：队列桥 `ProxyBridge` 实现 `proxy` 的 `ProxyHost`（清单 / 建会话 / 转达 / 观察 / 控制 / 消息都回到核心线程），以及代理工具面在代理会话里的挂载（`src/units/business/runtime/service/bridge.rs` 的 `inject_proxy_handler`）；工具面、授权判据与幂等账在 `proxy`（见 [../proxy/README.md](../proxy/README.md)），代理会话的装配在 `turn` 的 `build_proxy`，「建一个代理工作」的用例在 `work`。
- 管：**长步骤的边界**（见「一、长步骤的边界」）：生成、协作泵、压缩、起 / 停 guest 都不在队列里跑，队列只占「取 / 交」两步；建会话 / 启用历史会话 / 改配 / 删会话的 guest 启停在调用方线程上（起停对象在 `src/units/business/work/service/guest_job.rs`）。
- 管：动作分发（`src/units/business/runtime/service/action.rs`）：参数校验 → 按 `callers` 授权 → 执行 → 审计。
- 管：**历史生命周期的时机**——回档三模式、历史列表 / 只读打开与删除的调用点在这里（用例与顺序在 `lifecycle`，见 [../lifecycle/README.md](../lifecycle/README.md)）。
- 管：**同名薄转发**——工作用例（建工作两段、改配、视图、运行报告、建议、激活）的实现在 `work`，这里保留同名方法让队列与呈现层的调用点不变。
- 管：把各单元的入站面装配成呈现层要的几个窄面（`src/presentation/faces.rs` 的 `Faces`）。
- 运行态分两层：**短暂**的「这一刻谁在干活」（`SessionEvent::Working`，不落盘）与**持久**的「还该不该被驱动」（`meta.run`：`active` / `stopped` / `archived`，派发前过 `dispatch_gate`）。
- 不管：不读文件（`std::fs`）、不发网络（ureq）、不碰 stdin/stdout——机制一律下沉各单元的 `detail/`（见 `src/units/business/runtime/service/core.rs`）；不持任何别人的端口（R12）；不替用户选人、选形态。
- 不管：**工作是什么样**——工作形态的判据、建工作两段、改配、视图与运行报告在 `work`（见 [../work/README.md](../work/README.md)）。
- 不管：**用例的身体**——本单元的方法体只允许一行 `self.call(...)` 或一行转发（见「二、三条护栏」，实现面在 `src/units/business/runtime/service/contracts.rs`）。

## 入站契约与状态归属

- 入站面：`api::RuntimeOps`（启用 / 改需求 / 回档 / 停止）、`work::api::WorkOps`（工作用例与核心推荐）、`turn::api::TurnOps`（回合推进）、`api::ActionOps`（动作目录与分发）、`api::LogOps`（埋点门面）；`api::Handle` 是自持线程 + 命令队列 + 事件台的手柄，由 `src/presentation/faces.rs` 拆面后交给呈现层。
- 状态归属：命令队列只有它写；会话在世表、运行态、创建占位、裁决队、shell 槽、事件台与落盘游标归 `turn`，它只经 `turn` 的 `api` 读写。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`agentloop`、`catalog`、`collab`、`conversation`、`exec_model`、`kernel`、`lifecycle`、`llm`、`maintenance`、`permission`、`prompt`、`provision`、`proxy`、`registry_model`、`residents`、`session_model`、`tools`、`transcript`、`turn`、`turn_state`、`vm`、`work`、`workspace`
- 谁在用我：`presentation`

## 一、长步骤的边界

- 核心线程**只做毫秒级的状态变换**；凡是等外部世界的步骤都要 own-and-return（取走对象 → 在工作线程或调用方线程上跑 → 结果作为事件回来）。
- 生成 / 协作泵 / 压缩 / 节点：`turn` 的生成入口自己的线程（`solomni-gen` / `solomni-collab` / `solomni-compact` / `solomni-node` …），由队列手柄点火（`src/units/business/runtime/service/handle.rs`）。
- 起 / 停 guest：分两段——`prepare_*`（核心：校验、占名、出计划、建工作区）与 `commit_*`（核心：落盘、装配会话、开场事实），中间那段（`src/units/business/work/service/guest_job.rs` 的 `GuestBoot` / `GuestStop`）在调用方线程上跑；起一台虚拟机会话时核心照常应答，别的会话、状态查询、配置读取都不排队，失败则在提交前撤销（占名与空工作区目录都不留）。
- 启用历史会话：`activate_session` 同样两段——核心读元信息 / 判已归档 / 准备 guest，调用方线程起 guest，核心提交（重建会话、解冻整棵子树）；启用本身不跑模型，失败不留「已解冻 / 已装进内存」的半成品（`src/units/business/runtime/service/activation.rs`）。
- 删除会话：核心做完撤授权 / 回收常驻服务 / 移出内存并收集待删目录（毫秒级）；guest 销毁与目录删除交给调用方线程按序执行——先停 guest，再删目录（Windows 上 guest 可能还占着目录里的文件）。

## 二、三条护栏

- 护栏一·只向下依赖：本单元的依赖边只指向更内环的单元，环号在 `layers.yaml`，由 `tests/gates/structure/layers.js` 判红。
- 护栏二·谁都不许反向引用：任何单元不得 `use crate::units::business::runtime`（含全路径 `crate::units::business::runtime::…`）；反向边由 `tests/gates/structure/layers.js` 判红，本单元自己内部的 `crate::units::business::runtime::` 边被跳过。
- 护栏三·只编排不搬身体：本单元的方法体只允许一行 `self.call(...)` 或一行转发，出现 `if` / 循环 / 校验就是把用例放错了层——用例身体在 `work` / `turn` / `lifecycle` / `proxy`（`src/units/business/runtime/service/contracts.rs`）。

## 三、呈现层的取面方式

- 呈现层按屏幕取面，不再有一个装着全部会话命令的桶：`Faces` 的每个字段都是某一单元自己的窄契约或值（`src/presentation/faces.rs`）。
- 根只装配、不新增判定：`src/main.rs` 造出手柄后补上 `provision` / `storage` / `toolchains` 三个由组合根注入的面，其余字段由 `Faces::from_handle` 一次装好。
- 契约面名单以 `src/units/business/runtime/api.rs` 与各单元的 `api.rs` 为准；路由与屏幕的对应在 [docs/presentation/contracts.md](../presentation/contracts.md)。

## 改动本单元时必须同步

- `src/units/business/runtime/api.rs` 的契约方法 → 实现 `src/units/business/runtime/service/contracts.rs` 与 `src/tests/routes.rs` 的假实现一起改（路由表由 `src/tests/routes.rs` 机器比对）。
- `src/presentation/faces.rs` 的字段 → `src/main.rs`、`src/presentation/`、`docs/presentation/contracts.md` 一起改；增删字段要重跑 `src/tests/routes.rs`。
- 回档的时机与本单元的实现 → `lifecycle` 的用例与 `session` 的纯行算术一起改（见 `docs/lifecycle/README.md` 与 `docs/session/session-model.md` 的「回档：三种模式 + 工作级标记」）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
