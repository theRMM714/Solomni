# session（会话）

> 会话状态、回合簿记与会话历史的唯一持有者。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：会话状态与簿记（行 / 回合 / 回复）、成员工具面与工具表、历史视图与元信息、回档的纯行算术、呈现侧事件契约（`SessionEvent` / `LineView`）、核心操作回路（声明工具面 → 跑一次模型 → 取载荷 → 只读核实）。

**不管**：不编排跨会话（`conductor`）、不决定讨论流程（`collab`）、不实现工具机制（`tools`）。

## 二、入站契约与状态归属

`api::HistoryOps`（呈现层队列面）+ `api::History`（别的能力的直连面：造会话 / 写元信息 / **只读元信息** / 追流水 / 读回 / 删除）+ 行与事件词汇与**运行态**（`RunState`：暂停 / 关闭，落盘在 `meta.run`）。出站端口 `HistoryStore` **只由 `service.rs` 持有**（R12）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`prompt`、`taskchain`、`tools`、`workspace`
- 谁在用我：`collab`、`conductor`、`registry`、`slate`

## 四、改动本单元时必须同步

- 行与事件词汇（`domain/events.rs`、`LineView` 字段）一改 → `collab` 的状态派生、`conductor` 的视图、`cli`/`web` 渲染、`src/tests/session.rs` 全跟。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`session-model.md`](session-model.md) | 主/子会话、回合、发言标记、回档同步、上下文压缩 |