# lifecycle（会话历史生命周期）

> 定位：盘上会话历史的读侧与删除收尾——树序列表、只读打开、先停 guest 再删目录、后台按租约撤围栏。
> 效力：规范｜核对：2026-11-15｜依据：`src/capabilities/lifecycle/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：历史读侧——`api::list` 把盘上会话排成**树序**（父在前、子紧跟父），读盘失败记日志后返回空表；`api::open` 返回元信息与 rewind 截断后的事件流。
- 管：纯派生——`api::tree_order`（顶层与子层同一口径的时间倒序）与 `api::subtree_of`（父子关系只认 `meta.parent`）。
- 管：删除收尾段——`api::SessionDelete` 先跑调用方注入的停 guest 闭包，再**先子后父**删会话目录，最后 `api::FenceRelease` 在后台线程按租约撤权。
- 不管：会话对象、会话存储与事件流截断本身（`session` 的 `History` 端口与 `truncate_events`）。
- 不管：删前的内存动作与装配——回收常驻服务、移出内存、`forget_desk`、收集 guest 租约与子树运行态判定都在 `conductor` 的 `delete_bookkeeping`。
- 不管：围栏授权台账本身（`tools` 的 `release_by_lease`）；本单元只持调用方注入的撤销回调。

## 入站契约与状态归属

- 入站面：`api::list` / `api::open` / `api::tree_order` / `api::subtree_of`，以及收尾类型 `api::SessionDelete` / `api::FenceRelease` / `api::ReleaseByLease`；别的业务与测试只经 `api` 取用（R1）。
- 状态归属：**不持任何落盘状态**——会话与事件流归 `session`，围栏授权归 `tools`；`SessionDelete` 只装一次收尾所需的目录清单、目标会话与撤权回调，`run` 完即弃。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`session`（`History` / `HistoryView` / `SessionMeta` / `truncate_events`）、`kernel`（`ports::Log`）
- 谁在用我：`conductor`

## 一、删除收尾的三段顺序

- 先停 guest：`src/capabilities/lifecycle/service/delete.rs` 的 `run` 头部执行调用方注入的闭包；Windows 上 guest 还占着会话目录里的文件，顺序反过来就是 sharing violation。
- 再删目录：按调用方给的**先子后父**清单逐个 `history.delete`，删到一半失败即早退，失败如实返回。
- 最后撤权：`FenceRelease::spawn` 只起线程，撤权在后台按租约逐个做；线程起不来就同步兜底，授权不会漏撤。

## 改动本单元时必须同步

- `src/capabilities/conductor/service/history.rs` → `delete_bookkeeping` 是本单元唯一的装配点：停 guest 闭包、目录清单与租约清单的形状变化必须同改。
- `src/capabilities/lifecycle/service/delete.rs` → `SessionDelete::new` / `FenceRelease::new` 是唯一装配入口；改参数或 `run` 的三段顺序会同时影响 `src/tests/session/lifecycle.rs` 的五条收尾用例。
- `src/capabilities/lifecycle/api.rs` → 新增入站项要同步 [`unit-map.md`](unit-map.md) 与本页「入站契约」。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
