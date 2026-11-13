# cli（终端转录中心）

> 定位：argv → 入站能力面 → 渲染事件流。
> 效力：规范｜核对：2026-11-06｜依据：`src/presentation/cli.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../../TESTING.md)。

## 管什么 / 不管什么

- 管：终端转录中心——argv → 入站能力面 → 渲染事件流；`split_names` 与空登记处的引导文案是它自己的传输侧的事。
- 管：终端呈现本身——TTY 与档位判定、raw mode 的开关与恢复、冰青字标启动页、固定输入框（行编辑、历史、宽字符宽度）与状态行。交互模式只画这些；非 TTY（管道）保持逐行输入与原样输出，脚本口径不变。
- 管：交互模式的转录渲染——角色行与正文缩进、工具行（标签 + 参数摘要 + 成败）、验收清单与交付结论、裁决卡边框、生成态 spinner 与耗时、上下文用量写回状态行；同一批 `SessionEvent` 在非 TTY 下仍走原样文本。
- 管：共享区提交历史与回滚入口——`work <会话>` 看 head + 最近提交，`work <会话> log [条数]` 列提交，`work <会话> show <提交号>` 看详情，`work <会话> restore <提交号> [--yes]` 走统一的 `restore_work_commit` 动作（非破坏性；交互模式要求输入 yes，`--yes` 仅用于显式脚本）。
- 管：虚拟机档会话的只读诊断入口——`diagnose <会话>` 造一次 `guest_diagnose` 动作（`caller = user`），把动作返回的结构化 JSON（guest 进程表 / CPU 汇总 / 内存 / 负载 / 挂载）原样打印；与 `work` / `resident` / `module` 同类，不做业务判断。
- 不管：不做业务判断、不持状态、不落盘；**永不接触端口对象，也拿不到 `Core` 本身**（`src/presentation/cli.rs`）。

## 入站契约与状态归属

- 入站面：只依赖**入站能力面**（各能力的 `::api`）；`conductor` 的 `Ops` 是它与核心之间的唯一通道。
- 入口：`src/main.rs` 调 `presentation::cli::run(ops, presentation::web::DEFAULT_PORT)`；`webui` 子命令把控制权交给 Web 渠道（同一份 `Ops`）。
- 状态归属：不持状态、不落盘；会话与业务状态都在核心，本层只读 `Ops` 回报。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`conductor`、`modules`（经 `Ops` 字段）、`registry`、`vm`
- 谁在用我：（无）

## 改动本单元时必须同步

- 入站契约（`Ops` 的方法与事件）一改 → 本处渲染与 `src/tests/cli.rs`。
- 测试缺口：本单元**不是业务能力**，没有独立的缺口账；缺口记在 `tests/gaps.yaml`；格式见 [docs/testing/gaps-acceptance.md](../../../docs/testing/gaps-acceptance.md) 的「缺口账」。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |