# exec_model（执行与作业模型）

> 定位：纯领域——执行档位、执行选型与长任务进度这三类共享词汇的唯一家。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/exec_model/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：`Tier`（本机 / 虚拟机）、`ExecSpec`（档位 + 虚拟根名 + 能力定版）与 `JobState`（长任务进度快照）三个形状，以及档位的对外解析。
- 不管：虚拟机怎么起、清单怎么解析（`vm`）；执行计划怎么排、作业怎么推进（`provision` / `maintenance` / `runtime` 各自一段）。
- 不管：落盘。进出会话 `meta.yaml` 的 `exec` 段由会话侧读写，本单元只提供形状与解析。
- 不管：不认业务、不碰 IO、不持端口、不持有状态——环 1 的判据见 [layers.md](../architecture/layers.md) 的 H2。

## 入站契约与状态归属

- 入站面：`api`——重导出 `Tier` / `ExecSpec` / `JobState`。
- 出站端口：无（纯领域，不自持端口）。
- 状态归属：不持有状态；三个值随调用方传入与返回。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：无（环 1，只依赖标准库与 serde）。
- 谁在用我：`vm`、`provision`、`maintenance`、`system`、`runtime`、`turn`、`work`、`workspace`、`conversation`、`presentation`。

## 一、为什么是独立单元

- 三个形状原先挂在 `src/kernel/contracts/types.rs`：它们不含端口载荷，kernel 内部也没有使用点（`kernel::ports` 用的是 `Ask` / `AskOutcome` / `ToolOutcome`）。留在 kernel 等于让「产品词汇」占着地基库，且共享形状没有唯一的业务归属。
- 拆出来以后：kernel 只托管端口载荷（R6 的新口径），这三个形状归环 1，任何单元都只经 `units::exec_model::api` 取，不许再复制副本（R15）。

## 改动本单元时必须同步

- 改字段或序列化（`meta.yaml` 的 `exec` 段、进度通道的 `camelCase`）→ 会话 `meta` 读写方、`docs/session/session-model.md` 与进度通道的两个产生方（`provision` / `maintenance`）。
- 改 `Tier::parse` 的对外词汇 → 登记处与呈现层的档位入口、`REGISTRY_SPEC.md`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
