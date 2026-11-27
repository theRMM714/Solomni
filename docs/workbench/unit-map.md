# 单元地图 · workbench

> 定位：workbench（工作台契约）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-27｜依据：`src/units/business/workbench/api.rs`

> 本文是 **workbench** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/workbench.rs` | 单元根：只写目的 / 管 / 不管 / 联动与 `api` / `domain` 的 mod 声明 |
| `src/units/business/workbench/api.rs` | 入站面：工作面形状（`Area` / `Bench` / `Benches`）、寻址根与用量（`WorkRoots` / `WorkFiles` / `AreaUsage` / `WorkUsage` / `safe_file_name`）、过滤名单（`filter` 一族）、门禁形状（`VersionRef` / `Need` / `GateFault` / `GateItem` / `WriteGate`）与读取事实 `ReadFact` |
| `src/units/business/workbench/domain.rs` | 纯定义的入口：声明 `area` / `bench` / `benches` / `filter` / `gate` / `readfact` / `roots` 七个概念子模块 |
| `src/units/business/workbench/domain/gate.rs` | 写入门禁的形状：某个路径的某一版 `VersionRef`（指纹 + 可选字节数）、判据 `Need`（观察过 / 全文）、四种拒绝 `GateFault`（含 `code` 与给模型看的文案）、逐条输入 `GateItem` 与判定契约 `WriteGate`（`check` / `check_many`，默认逐条走到第一条拒绝） |
| `src/units/business/workbench/domain/area.rs` | `Area`：一席看到的路径先归到哪一区（共享 / 私有 / 模块）；形状进契约单元，具体布局由 `workspace` 铺 |
| `src/units/business/workbench/domain/bench.rs` | 一席工作面 `Bench`：解析路径到区、问可读可写、拒写文案、相对路径与根清单；字段即布局事实（工作名 / 会话 / 私沙根 / 模块目录 / 权限面 / 共享区归属 / 路径翻译器 / 过滤名单路径），不认识版本库 |
| `src/units/business/workbench/domain/benches.rs` | 一次工作的全部工作面 `Benches`（共享区根 / 会话根 / 每席一份 `Bench` / 路径翻译器），按 agent 取 |
| `src/units/business/workbench/domain/filter.rs` | 工作区过滤的唯一名单与匹配规则：`.workignore` 的文件名与播种内容、把规则文本编译成与 git 同引擎的匹配器（`Rules`）、按相对路径判定忽略（命中的目录连带整棵子树）；内容面与检索面都从这里取规则 |
| `src/units/business/workbench/domain/roots.rs` | 寻址根与用量（`WorkRoots` / `WorkFiles` / `AreaUsage` / `WorkUsage`）与文件名净化 `safe_file_name` |
| `src/units/business/workbench/domain/readfact.rs` | 读取事实 `ReadFact`：哪个工作树、哪个路径、读到哪几行、当时的整份指纹（随工具行落盘，过期由它派生） |
