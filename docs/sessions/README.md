# sessions（会话盘资源）

> 定位：`data/session` 这块盘的唯一主人——会话根归谁拼、某个会话在不在盘上。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/resource/sessions/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)。
> 与 [session/（会话业务）](../session/README.md) 的分工：那边管**会话是什么**（身份、meta、转录语义），本单元只管**那块盘在哪、有没有**。

## 管什么 / 不管什么

- 管：会话根（`data/session`，由组合根经 `layout::session` 注入）、会话目录的存在性查询（`exists`）。
- 不管：会话目录内部布局与文件格式——转录是 `conversation` 的 `FsHistory`（`<根>/<会话>/meta.yaml` + `transcript.jsonl` + `children/`），共享区与沙箱是 `workspace` 的 `FsWorkspace`（`<根>/<会话>/work/`、各 agent 沙箱、`.work-root`）。
- 不管：会话名合不合法（`workspace::api::safe_file_name` 与 `runtime` 的调度）。
- 不管：介质名与路径常量（唯一权威是 `src/kernel/domain/layout.rs`）。

## 入站契约与状态归属

- 入站面：`api`（`Sessions`）。
- 出站端口：`SessionDirs` **只由 `service.rs` 持有**（R12）；真实实现 `detail::FsSessionDirs` 只由组合根构造。
- 状态归属：**无内存状态**——会话根是构造时定下的一个值，存在性每次都问盘。
- 一个根、两个消费者：转录与共享区**共用**会话目录 `<根>/<会话>/`，各写互不重叠的子路径（`meta.yaml` / `transcript.jsonl` / `children/` 对 `work/` / 沙箱 / `.work-root`）。本单元不把两种实现合并，也不给两个根——盘上只有一处会话目录。
- 路径不外泄：`root()` 只由组合根交给 `FsHistory` / `FsWorkspace` 两个文件适配器；业务 service 拿不到路径，只拿 `exists` 的事实（R16）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`（只有 `layout` 的路径存在性判定的使用方在组合根）
- 谁在用我：`system`（`work_gc` 先问"这个工作有没有"）、`entry`（组合根装配转录与共享区适配器）

## 一、盘面

| 落点 | 谁写 | 内容 |
| --- | --- | --- |
| `data/session/<会话>/` | `sessions`（建目录的判定）、`conversation` | 会话目录本身 |
| `data/session/<会话>/meta.yaml` | `conversation` 的 `FsHistory` | 会话元信息 |
| `data/session/<会话>/transcript.jsonl` | `conversation` 的 `FsHistory` | 事件流水（只追加） |
| `data/session/<会话>/children/<子会话>/` | `conversation` 的 `FsHistory` | 子会话（任意深度） |
| `data/session/<会话>/work/` | `workspace` 的 `FsWorkspace` | 共享区 |
| `data/session/<会话>/<agent>/` | `workspace` 的 `FsWorkspace` | 各 agent 沙箱 |
| `data/session/<会话>/.work-root` | `workspace` 的 `FsWorkspace` | 外部工作区绑定的布局投影 |

- 扫描只读：`maintenance` 的存储盘点会统计这棵树的用量（[storage/README.md](../storage/README.md)），只读不写、不删。

## 二、为什么存在性查询要从业务层收上来

- `system` 的 `work_gc` 要先判"这个工作存在吗"再决定删不删。这在业务层里曾是 `layout::session(root).join(work).is_dir()`——业务层直接碰文件系统，既绕过了盘的主人，也让"会话盘在哪"这一知识多出一份。现在它只问 `Sessions::exists`，路径知识留在本单元。

## 改动本单元时必须同步

- 新增 / 改动 `Sessions` → `system` 的 `work_gc` 与 [system/README.md](../system/README.md)、[storage/README.md](../storage/README.md) 的盘点口径。
- 会话目录内部布局的改动 → [session/session-model.md](../session/session-model.md) 与 [workspace/README.md](../workspace/README.md)（不在本页）。
- 端口形状改动 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
