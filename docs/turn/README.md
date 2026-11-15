# turn（会话在世状态）

> 定位：会话在世表、运行态、占名表、裁决队句柄与 shell 状态槽的唯一持有者——只给 `conductor` 提供访问器。
> 效力：规范｜核对：2026-11-15｜依据：`src/capabilities/turn/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：会话在世表——`SessionId → Session` 的增删查与遍历（`put` / `take` / `session` / `session_mut` / `holds` / `iter`）。
- 管：运行态——「对象被工作线程取走了、正在生成」的标记集合（`mark_running` / `is_running` / `running_iter` / `any_running`）。
- 管：占名表——准备段与提交段之间占下的名字（`mark_starting` / `is_starting` / `unmark_starting`）：同名并发创建不会都通过「名字不存在」那道检查而互相覆盖。
- 管：裁决队句柄与 shell 状态槽——`Arc<DecisionDesk>` 按会话 id 取（`desk`），shell 槽按会话 id 惰性造一份、同一会话共享（`shell_slot` / `shell_view`）。
- 不管：会话对象怎么建、回合怎么跑（`conductor`）；会话与转录怎么落盘（`session`）；裁决队的语义（`session` 的 `DecisionDesk`）与 shell 快照的形状（`session` 的 `ShellSlot` / `ShellLive`）。
- 不管：命令队列与派发闸（`conductor` 的 `service.rs`）；本单元只按 id 记账，不认识会话的内容。

## 入站契约与状态归属

- 入站面：`api::TurnState`（全部访问器）与 `api::Session`（`Single` / `Collab` 两变体）；别的业务与测试只经 `api` 取用（R1）。
- 状态归属：**持全部运行期内存态**——五张表都在 `TurnState` 里；不落盘，`shell_slots` 的 `Mutex` 只在取 / 写那一刻持有。
- 裁决队是 `Arc<DecisionDesk>`：生成线程与核心线程共用同一条队，所以队不随会话对象被取走而消失（工具级确认正是在「生成中」问的）。
- 运行态与占名表分开记：前者表示「对象在生成、不在表里是正常的」，后者表示「名字已占、会话还没落盘」。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`collab`（`CollabSession`）、`kernel`（`SessionId`）、`session`（`AgentSession` / `DecisionDesk` / `ShellLive` / `ShellSlot`）
- 谁在用我：`conductor`

## 一、五张表各管什么

- 在世表：会话对象在核心手里时在里面；被工作线程取走时不在（生成中），取走与交回都经 `take` / `put`。
- 运行态：只记 id，不持对象；用于「整棵子树有生成中就拒绝」这类判定（删会话、回档、改配、切换档位前都要问它）。
- 占名表：只记名字，用于建工作的两段之间挡同名并发；提交成功或失败撤销时都要撤掉。
- 裁决队：句柄随会话 id 走，`Arc` 让等待方与改写方看同一条队；会话消失时由 `conductor` 调 `desk().forget`。
- shell 槽：工具循环写、状态栏读；同一会话拿到的是同一个 `Arc<ShellSlot>`，没跑过 shell 时快照如实为空。

## 改动本单元时必须同步

- `src/capabilities/turn/api.rs` → 新增入站项要同步 [`unit-map.md`](unit-map.md) 与本页「入站契约」。
- `src/capabilities/conductor/service.rs` → `Conductor` 的 `turn` 字段是唯一持有者；改访问器签名会连带改 `service/` 下的调用方（`activation` / `flow` / `history` / `proxy` / `rewind` / `turn` / `work/config` / `work/create` / `work/model` / `work/views`）。
- `src/tests/turn.rs` → 状态容器的三条用例：运行标记按 id 收放、占名与在世表互不影响、shell 槽按会话共享。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
