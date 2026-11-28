# lifecycle（会话历史生命周期）

> 定位：会话历史做什么——历史列表与只读打开、删除的报账与收尾、回档三模式（留档 / 删除 / 还原）。
> 效力：规范｜核对：2026-11-28｜依据：`src/units/business/lifecycle/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**回档三模式**（`rewind` / `rewind_outcome`）——留档（写标记 + 折叠子会话）、删除（收窗子会话 + 截断）、还原（删标记及其后 + 重放子会话）。
- 管：**会话重建**（`restore_session`）：按 `turn` 的 `rebuild_session` 重建并放回会话，新行续号取全量最大 id + 1、永不回退。
- 管：**历史列表与只读打开**（`history_list` / `history_open`）：列表按树序派生；打开时取内容日志的活动视图（`Content::folded_away`，见 [docs/transcript](../transcript/README.md)）。
- 管：**删除的报账与收尾**（`delete_bookkeeping` / `SessionDelete::run`）：撤权的先后、常驻服务回收、移出内存与待删目录的收集；收尾段先停 guest、再删目录、最后后台撤围栏授权。
- 管：**回档标记的续号**（`next_mark_id`）与**按 agent 收窗**（`keep_by_agent`）：留档 / 删除时各会话保留到哪一行。
- 不管：行与事件的算术、留档标记的解析（`agentloop` 的 `domain::rewind`）与落盘机制（`session` 的 `History` 端口）。
- 不管：命令队列、生成驱动与事件台——谁在什么时候回档 / 删除、结果推给谁，由 `runtime` 决定。
- 不管：沙箱与版本化机制（`workspace`）、虚拟机的起停实现（`vm` 的 `GuestStop`）；本单元只按用例顺序调用它们。

## 入站契约与状态归属

- 入站面：`api` 的 `RewindTarget`（留档 / 删除按**行 id**，还原按**标记 id**）、`RewindOutcome`（主会话活动视图 + 受影响的子会话视图）与 `RebuildDeps`（重建要的登记处与探针，按次传入）。
- 删除的收尾对象：`SessionDelete`（`run` 在调用方线程上跑，返回「目标会话是否删成功」）与撤权回调 `ReleaseByLease`。
- 状态归属：`Lifecycle` 只持其它单元的入站面（`history` / `workspace` / `tools` / `vm` / `residents` / `log`），**不自持会话状态**：会话本体、在世表与运行态归 `turn`，经形参 `&mut Turn` / `&Turn` 读写；`registry` / `probe` 经 `RebuildDeps` 按次传入（R12）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`agentloop`、`collab`、`kernel`、`residents`、`session_model`、`tools`、`turn_state`、`vm`、`workspace`
- 谁在用我：`runtime`
- **不引用 `runtime`**：依赖只向下，`lifecycle` 在 `runtime` 之下（`layers.yaml`）。

## 一、回档三模式的顺序

- 留档：先算各子会话折叠点（不落盘），再算共享区点——两者都成功才动文件；**文件已就位，最后写转录标记**（顺序反了会出现「标记说已留档、文件还在新枝」）。主标记写完后逐个折叠子会话，最后重建主会话。
- 删除：不带 `force` 时先按待删标记要求确认；再收窗子会话、删工作区提交、截断转录，最后就地回退或重建。
- 还原：先按标记恢复工作区提交，再截断转录，最后重放子会话。
- 三种模式都在尾部按 `tool_runs` 的前后差推一条提示：移出的工具执行副作用不在回档范围内。

## 二、删除的两段

- 核心段（`delete_bookkeeping`）：读元信息 → 拒绝单独删子会话、拒绝删除仍在运行的子树节点 → 逐个回收常驻服务租约 → 备好停 guest 的对象 → 移出内存并收齐待删目录（按子树由叶到根的逆序）。
- 收尾段（`SessionDelete::run`）：**先停 guest、再删目录**（Windows 上 guest 可能还占着目录里的文件）——任一目录删失败即停，最后把整棵子树的围栏撤权交给后台线程；撤权失败只进日志，不回滚已完成的删除。

## 改动本单元时必须同步

- `src/units/business/lifecycle/api.rs` 的 `RewindTarget` / `RewindOutcome` → `runtime` 的 `api.rs`（`RuntimeOps::rewind`、`rewind_with_reset`）与 `src/tests/api/session.rs` 一起改。
- `src/units/business/lifecycle/service/rewind.rs` → 回档的行 id、标记顺序、拒绝文案与提示文案是行为契约：改一处必须同改 `src/tests/session/rewind.rs`、`src/tests/runtime.rs` 的断言。
- `src/units/business/lifecycle/service/history.rs` → 删除的顺序、日志 key（`lifecycle::history_delete` / `lifecycle::release_fences`）与拒绝文案是行为契约：同改 `src/tests/session/lifecycle.rs`。
- `src/units/business/turn/turn/service/rebuild.rs` 的 `rebuild_session` → 本单元重建的唯一入口：改签名必须同改 `src/units/business/lifecycle/api.rs` 的 `RebuildDeps` 与两处调用点。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
