# workbench（工作台契约）

> 定位：一个会话「在哪儿干活」的契约——工作面形状与寻址、过滤名单、版本身份与写入门禁都在本单元，实现分别由策略给出。
> 效力：规范｜核对：2026-11-27｜依据：`src/units/business/workbench/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**地址与区域**——`Area`（共享 / 私有 / 模块）：一席看到的路径先归到区，再问这一区能不能读、能不能写；切分结构而不认识具体布局策略。
- 管：**工作面** `Bench`（一席的路径解析、可读可写判定、拒写文案、路径换算与根清单）与 **一次工作的全部工作面** `Benches`（共享区根 / 会话根 / 每席一份 / 路径翻译器）；布局由 `workspace` 铺，本单元只说形状与语义。
- 管：**写入门禁的契约** `WriteGate` / `GateItem` / `VersionRef` / `GateFault`：判定要什么（观察过的版本、全文读）、拒绝的四种形态与文案；`VersionRef` 是「某条路径的某一版」，不是 `versions` 的完整视图。
- 管：**读取事实** `ReadFact`：哪个工作树、哪个路径、读到哪几行、当时的整份指纹——它随工具行落盘，过期由它派生。
- 管：**过滤名单**——`.workignore` 的文件名与播种内容、与 git 同引擎的匹配器 `Rules`；内容面与检索面共用这一份（`src/units/business/workbench/domain/filter.rs`）。
- 管：**观察事实** `ObservationFact` / `Coverage`：工具行落盘的「读过没有、读到哪几行」，门禁与写回执都从这里取，不在别处重算。
- 管：**工作台策略的契约**——`BenchPolicy`（id / 能力位 / 共享区可不可写 / `plan` 开一席）、`BenchPlan`（工具 id、执行者、回合钩子 `BenchDriver`）与策略表 `BenchPolicies`；共享区版本化动作的整组名单 `SHARED_TOOLS` 与能力位 `VERSIONS_CAP` 也在这里，实现分在 `versions`（versioned）与 `workspace`（plain）。
- 不管：**怎么铺**（`workspace` 的布局策略）、**怎么版本化**（`versions` 的策略）、**怎么起进程**（`sandbox` 的策略与其实现 `vm`）。
- 不管：**谁被授权**（`session_model` 的规则链与授权端口 `Policy`）、**门禁怎么判**（实现在 `versions` 的 `SessionWriteGate`，它读版本库快照）、**哪个策略接进来**（组合根拼 `Policies`）。

## 入站契约与状态归属

- 入站面：`api`——`Area` / `Bench` / `Benches` / `WorkRoots` / `WorkFiles` / `AreaUsage` / `WorkUsage` / `safe_file_name` / `WORKIGNORE` / `Rules` / `VersionRef` / `Need` / `GateFault` / `GateItem` / `WriteGate` / `ReadFact` / `ReadQuery` / `Coverage` / `LineRange` / `ObservationFact` / `ObservationSource` / `OBSERVATION_FORMAT` / `BenchPolicy` / `BenchPolicies` / `Policies` / `BenchPlan` / `BenchDriver` / `PlanRequest` / `GateRequest` / `CallRequest` / `WriteDone` / `BatchDone` / `SHARED_TOOLS` / `VERSIONS_CAP` / `VERSIONED_POLICY` / `PLAIN_POLICY`。
- 本单元**无状态、无 IO**：只有形状与 trait；策略与实现都在别的单元（`src/units/business/workbench/domain/gate.rs`）。
- 为什么单独成一个单元：`tools` 执行内置工具要认工作面与门禁，而 `workspace` / `versions` 要实现它们；放进任何一方都会让 `tools` 与那一方成环。放在环 2 的独立契约单元后，两边都只指向它。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——只依赖 `kernel` 与环 1）。
- 谁在用我：`tools`（内置工具的寻址与读写判定）、`agentloop`（装配期持 `Bench`、按策略 `plan` 拿工具与回合钩子）、`versions`（`PathGate` 的版本字段、读取事实、过滤规则、`SessionWriteGate` 与 versioned 策略）、`workspace`（组装 `Benches`、plain 策略）、`turn` / `turn_state` / `collab` / `work`（流程里带 `Benches`、视图读能力位）。

## 改动本单元时必须同步

- `docs/session/session-model.md` → 「写门禁」一节的口径（观察过才准覆盖、压缩后失效）。
- `docs/tools/` → 内置工具写路径的拒绝文案与 `GateFault::code` 一一对应。
- `docs/versions/README.md` → `work_gate_info` 只回答快照三件事；判定在 `versions` 的 `SessionWriteGate`（经策略的 `BenchDriver::gate` 交给引擎），别把判定写成 `versions` 的对外职责以外的形状。
- `docs/workspace/README.md` → 策略表接哪几个、`plain` 与 `versioned` 的差别（单树可写 / 共享区只读）。
- `docs/tools/tools-and-roles.md` → 共享区动作由策略发放，工具面不硬编码 `work_*`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
