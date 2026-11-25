# system（系统维护与自检）

> 定位：系统维护与自检——把「这台机器能承载什么、占了多少钱、缺什么、怎么清」做成一份可执行的结论，供启动相命令与 Web 共用。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/system/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：自检报告——平台与架构、围栏事实、外部可执行、基础工具链、进程账、虚拟机档逐条要求、存储概览。
- 管：存储报告的对外形状与回收命令的语义分派（有类别按类别、无类别整体回收）。
- 管：依赖补齐的状态查询、补齐执行与虚拟根还原的确认门槛。
- 管：工作区回收与模块构建在维护视角下的前置校验（名字合法、工作存在、模块存在、目标与出网开关）。
- 管：便携维护（`--setup`）——用户 PATH 的 shim 追加 / 摘除 / 还原、只读状态视图、围栏机器级预授（预授成功顺手把授权记进设置）；写入结果按一句话回执交呈现层排版。
- 不管：回收与补齐的实现（归 `storage` / `provision`）、构建编排（归 `builds`）、工作区对象图（归 `workspace`）、模块清单（归 `modules`）、工具链条目解析（归 `src/entry/toolchains.rs`）。
- 不管：命令名、别名、分组与手册排版（归 `systools/commands.yaml` 与 `src/presentation/cli/catalog.rs`）；也不解析 argv（归 `src/presentation/cli/boot.rs`）。

## 入站契约与状态归属

- 入站面：`api::System` 的十个方法（自检 / 存储 / 回收 / 补齐状态 / 补齐 / 还原 / 工作区回收 / 构建 / 作业进度 / 便携维护）与 `api::Fault`——`code` 是退出码，`message` 是给人看的一句话，呈现层只打印它、按 `code` 退出。
- 便携维护另有 `api::SetupOps` 端口（`apply` / `fence_missing` / `fence_ensure`）与 `api::SetupMode` 的模式表：写入实现（注册表、shim、ACL）在 `src/units/system/detail/setup.rs`，`service` 只持端口；`api::FencePolicy` 是「预授成功后把授权记进设置」这一个动作的端口（`fence`/`all` 成功后才调，失败只降级成一行回执、不改退出码）；启动期对账（`src/entry/portable.rs`）也只经这些端口取缺项与预授。
- 出站端口：`storage` / `provision` / `builds` / `workspace` / `modules` / `sessions` 的 `::api`，加上装配期注入的基础工具链事实与出网开关，全部经 `SystemDeps` 注入。
- 状态归属：本单元不持有状态；自检是只读快照，回收 / 补齐 / 构建的状态归各自的 owner（`storage` 的作业与上次回收摘要、`provision` 的补齐进度、`builds` 的构建状态）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`catalog`、`exec_model`、`kernel`、`maintenance`、`provision`、`vm`、`workspace`
- 谁在用我：`entry`、`presentation`

## 一、命令面与执行面的分工

- 命令名、参数与确认门槛的唯一声明是 `systools/commands.yaml`；操作数由 `src/units/tools/domain/op.rs` 的 `parse` 按声明转成 JSON，`src/presentation/cli/boot.rs` 只按字段名取值、翻成 `api::System` 的一次调用并排版回执。
- 确认门槛由实现单元判，不靠命令行拦：`ensure` / `reset` 缺 `confirm` 报 `Fault::usage`；`gc` 无类别时缺省 dry-run（返回 0 并给预览），带类别时缺 `--yes` 由呈现层按预览退出 2——与迁移前的旗语行为逐条一致。
- 退出码口径：0 成功 / 1 执行失败 / 2 用法或确认门槛不对，由 `Fault` 的两个构造器区分。
- 状态视图：`setup status` 的回执是平台 / 根 / PATH 入口 / 围栏缺项的键值块，呈现层不带 `--json` 时排成三行（`[便携]` 前缀），带 `--json` 时原样打这份 JSON。
- 机器回执：呈现层加 `--json` 时只输出一段 JSON，字段逐命令写在 `systools/commands.yaml` 的 `json`（本单元给的是值，排版与“加法”在 `boot.rs`）；失败时 stderr 一行 `{"error","code","domain"}`，退出码与人工模式一致。

## 二、自检报告

- `doctor` 只读：围栏事实、外部可执行（python / node / curl）、基础工具链投影、进程账、虚拟机档逐条要求、存储概览。
- `doctor_with(fence)` 让调用方注入围栏事实：启动期复用同一次能力探测，自检与启动报告不会各问一遍（用例见 `src/tests/diagnostics.rs`）。
- 报告是机器可读契约：字段名与取值由 `src/tests/diagnostics.rs` 直接断言。

## 三、写操作的只有三条路

- `ensure`（下载并写进项目内）、`reset`（删虚拟根内状态后重建）、`gc`（删缓存与内容对象）——三条都要求 `confirm`，缺了如实报用法错，不静默降级成只读。
- `reset` 成功后顺带清该虚拟根的构建缓存；清缓存失败不影响还原结论。

## 改动本单元时必须同步

- `src/presentation/cli/boot.rs` → 新增或改启动相命令时同步操作数解析、回执与退出码。
- `systools/commands.yaml` → 命令名、分组、参数声明与确认门槛；装配期由 `src/main.rs` 的 `command_ops` 对声明做硬失败自检，表与两处分派的对齐由 `src/tests/commands.rs` 锁死。
- `src/entry/assembly.rs` → `SystemDeps` 的装配；新增端口注入只在这里落一次。
- `docs/entry/README.md` → 组合根不再执行产品命令；启动相清单以本页与命令表为准。
- 测试缺口：本单元是业务单元，缺口记在 `tests/gaps.yaml`；格式见 [docs/testing/gaps-acceptance.md](../testing/gaps-acceptance.md) 的「缺口账」。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
