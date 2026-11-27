# workbench（工作台契约）

> 定位：一个会话「在哪儿干活」的契约——工作区、一席的工作台、执行环境、写入门禁四张面都在本单元，实现分别由策略给出。
> 效力：规范｜核对：2026-11-27｜依据：`src/units/business/workbench/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**地址与区域**——`Area`（共享 / 私有 / 模块）与 `Address`：一席看到的路径先归到区，再问这一区能不能读、能不能写；切分结构而不认识具体布局策略。
- 管：**工作台契约** `Workbench`（一席的解析、可读可写判定、拒写文案、路径换算、门禁入口、工具面与处理器）与 **工作区契约** `Workspace`（一次会话铺出来的所有席、工具清单、占用、根、注销）。
- 管：**写入门禁的契约** `WriteGate` / `GateItem` / `VersionRef` / `GateFault`：判定要什么（观察过的版本、全文读）、拒绝的四种形态与文案；`VersionRef` 是「某条路径的某一版」，不是 `versions` 的完整视图。
- 管：**读取事实** `ReadFact`：哪个工作树、哪个路径、读到哪几行、当时的整份指纹——它随工具行落盘，过期由它派生。
- 不管：**怎么铺**（`workspace` 的布局策略）、**怎么版本化**（`versions` 的策略）、**怎么起进程**（`sandbox` 的策略与其实现 `vm`）。
- 不管：**谁被授权**（`session_model` 的规则链与授权端口 `Policy`）、**门禁怎么判**（实现在 `agentloop` 的 `SessionWriteGate`，它读 `versions` 的快照）。

## 入站契约与状态归属

- 入站面：`api`——`VersionRef` / `Need` / `GateFault` / `GateItem` / `WriteGate` / `ReadFact`。
- 本单元**无状态、无 IO**：只有形状与 trait；策略与实现都在别的单元（`src/units/business/workbench/domain/gate.rs`）。
- 为什么单独成一个单元：`tools` 执行内置工具要认门禁，而 `versions` 要实现门禁；若把这份契约放进任何一方，就会让 `tools` → `versions` 或反向成环。放在环 2 的独立契约单元后，两边都只指向它。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——只依赖 `kernel` 与环 1）。
- 谁在用我：`tools`（`run_builtin` 的 `gate` 形参）、`agentloop`（`SessionWriteGate` 实现 `WriteGate`，装配与判定用它）、`versions`（`PathGate` 的版本字段与读取事实）、`turn`（重建时把读取事实读回来）。

## 改动本单元时必须同步

- `docs/session/session-model.md` → 「写门禁」一节的口径（观察过才准覆盖、压缩后失效）。
- `docs/tools/` → 内置工具写路径的拒绝文案与 `GateFault::code` 一一对应。
- `docs/versions/README.md` → `work_gate_info` 只回答快照三件事；判定在 `agentloop`，别把它写成 `versions` 的职责。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
