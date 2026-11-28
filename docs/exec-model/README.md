# exec_model（执行与作业模型）

> 定位：纯领域——执行侧的「哪一边」与派生选型、以及长任务进度这三类共享词汇的唯一家。
> 效力：规范｜核对：2026-11-28｜依据：`src/units/pure/exec_model/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：`Side`（`host` / `guest` 哪一侧）、`TargetSpec`（侧 + 客机名 + 能力定版，由执行环境声明解出来的派生选型，不落盘）与 `JobState`（长任务进度快照）三个形状。
- 不管：虚拟机怎么起、清单怎么解析（`vm`）；执行计划怎么排、作业怎么推进（`provision` / `maintenance` / `runtime` 各自一段）。
- 不管：落盘与策略。会话 `meta.yaml` 的 `sandbox` 段（策略 id + 参数）归 `session_model` 存、归 `sandbox` 解释；本单元只提供从事实算出来的选型。
- 不管：不认业务、不碰 IO、不持端口、不持有状态——环 1 的判据见 [layers.md](../architecture/layers.md) 的 H2。

## 入站契约与状态归属

- 入站面：`api`——重导出 `Side` / `TargetSpec` / `JobState`。
- 出站端口：无（纯领域，不自持端口）。
- 状态归属：不持有状态；三个值随调用方传入与返回。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：无（环 1，只依赖标准库与 `serde`）
- 谁在用我：`catalog`、`maintenance`、`provision`、`residents`、`runtime`、`sandbox`、`session_model`、`system`、`tools`、`turn`、`vm`、`work`、`workspace`

## 一、为什么是独立单元

- 三个形状原先挂在 `src/kernel/contracts/types.rs`：它们不含端口载荷，kernel 内部也没有使用点（`kernel::ports` 用的是 `Ask` / `AskOutcome` / `ToolOutcome`）。留在 kernel 等于让「产品词汇」占着地基库，且共享形状没有唯一的业务归属。
- 拆出来以后：kernel 只托管端口载荷（R6 的新口径），这三个形状归环 1，任何单元都只经 `units::pure::exec_model::api` 取，不许再复制副本（R15）；「哪一侧」只有这一份，执行环境策略不另立。

## 改动本单元时必须同步

- 改字段或序列化（进度通道的 `camelCase`）→ 进度通道的两个产生方（`provision` / `maintenance`）。
- 改 `Side` 的对外词汇 → 执行环境策略的 `facts`（`sandbox` / `vm`）、提示词与回执里说「本机 / 客机」的地方。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
